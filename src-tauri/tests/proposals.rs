//! Integration tests for the typed proposal engine: propose → apply → undo,
//! the read-only invariant, TTL expiry, version conflicts, hostile model
//! output, and the audit trail. No test touches the network or a real keychain.

use std::sync::Mutex;

use chrono::Duration;
use contractorcrm_lib::ai::{
    set_ai_settings, CompletionProvider, InMemoryCredentialStore, ProviderCheck,
    ProviderCompletion, ProviderRequest, SetAiSettingsRequest,
};
use contractorcrm_lib::application::{
    self, CompanyPatch, ContactPatch, CreateCompanyRequest, CreateContactRequest,
    CreateOpportunityRequest, OpportunityPatch, UpdateContactRequest,
};
use contractorcrm_lib::domain::Actor;
use contractorcrm_lib::error::ApplicationError;
use contractorcrm_lib::proposals::{
    apply_proposal, propose_record_with_provider, propose_update_with_provider, undo_proposal,
    ApplyProposalRequest, ProposalEntityType, ProposalKind, ProposalStore, RecordVersion,
    UndoProposalRequest,
};
use contractorcrm_lib::storage::Storage;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// A provider that answers with canned text — the deterministic machinery under
/// test must not depend on a real model.
struct CannedProvider {
    answer: String,
    last_request: Mutex<Option<ProviderRequest>>,
}

impl CannedProvider {
    fn new(answer: &str) -> Self {
        Self {
            answer: answer.to_owned(),
            last_request: Mutex::new(None),
        }
    }

    fn last_request(&self) -> ProviderRequest {
        self.last_request
            .lock()
            .expect("request mutex")
            .clone()
            .expect("provider was called")
    }
}

impl CompletionProvider for CannedProvider {
    fn label(&self) -> &str {
        "Canned model"
    }

    fn complete(&self, request: &ProviderRequest) -> Result<ProviderCompletion, ApplicationError> {
        *self.last_request.lock().expect("request mutex") = Some(request.clone());
        Ok(ProviderCompletion {
            purpose: request.purpose.clone(),
            model: "canned".into(),
            text: self.answer.clone(),
            included_record_refs: request.included_record_refs.clone(),
        })
    }

    fn check(&self) -> Result<ProviderCheck, ApplicationError> {
        Err(ApplicationError::ProviderUnavailable {
            reason: "connection checks are not part of these tests".into(),
        })
    }
}

fn open_storage(temp: &tempfile::TempDir) -> Mutex<Storage> {
    Mutex::new(Storage::open_in_app_data(temp.path()).expect("open storage"))
}

fn contact_patch(display_name: &str) -> ContactPatch {
    ContactPatch {
        display_name: Some(display_name.into()),
        kind: "client".into(),
        ..ContactPatch::default()
    }
}

fn seed_contact(storage: &Mutex<Storage>, display_name: &str) -> String {
    let mut guard = storage.lock().expect("storage lock");
    application::create_contact(
        &mut guard,
        CreateContactRequest {
            actor: Actor::User,
            contact: contact_patch(display_name),
        },
    )
    .expect("seed contact")
    .id
}

fn seed_opportunity(storage: &Mutex<Storage>, contact_id: &str, name: &str) -> String {
    let mut guard = storage.lock().expect("storage lock");
    application::create_opportunity(
        &mut guard,
        CreateOpportunityRequest {
            actor: Actor::User,
            stage_id: None,
            opportunity: OpportunityPatch {
                name: name.into(),
                contact_id: Some(contact_id.to_owned()),
                value_minor: 100_000,
                currency_code: "USD".into(),
                ..OpportunityPatch::default()
            },
        },
    )
    .expect("seed opportunity")
    .id
}

/// Cheap content fingerprint: row counts and the sum of every record version,
/// so any insert, update, or delete moves the number.
fn fingerprint(storage: &Mutex<Storage>) -> Vec<i64> {
    let guard = storage.lock().expect("storage lock");
    let connection = guard.connection();
    [
        "SELECT COUNT(*) FROM contacts",
        "SELECT COALESCE(SUM(version), 0) FROM contacts",
        "SELECT COUNT(*) FROM contact_channels",
        "SELECT COUNT(*) FROM companies",
        "SELECT COALESCE(SUM(version), 0) FROM companies",
        "SELECT COUNT(*) FROM opportunities",
        "SELECT COALESCE(SUM(version), 0) FROM opportunities",
        "SELECT COUNT(*) FROM activities",
        "SELECT COUNT(*) FROM tasks",
        "SELECT COUNT(*) FROM command_log",
    ]
    .iter()
    .map(|sql| {
        connection
            .query_row(sql, [], |row| row.get::<_, i64>(0))
            .expect("fingerprint query")
    })
    .collect()
}

fn command_log(storage: &Mutex<Storage>) -> Vec<(String, String, String)> {
    let guard = storage.lock().expect("storage lock");
    let connection = guard.connection();
    let mut statement = connection
        .prepare("SELECT actor, entity_type, summary FROM command_log ORDER BY created_at, id")
        .expect("prepare command log");
    let rows = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .expect("query command log")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect command log");
    rows
}

// ---------------------------------------------------------------------------
// Propose → apply
// ---------------------------------------------------------------------------

#[test]
fn a_drafted_contact_applies_through_the_ordinary_create_path() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let store = ProposalStore::new();
    let provider = CannedProvider::new(
        r#"{"firstName":"Dana","lastName":"Ruiz","kind":"client","phone":"555-0100","city":"Sanford"}"#,
    );

    let proposal = propose_record_with_provider(
        &storage,
        &provider,
        &store,
        ProposalEntityType::Contact,
        "New client Dana Ruiz in Sanford, 555-0100",
    )
    .expect("draft a contact");

    assert_eq!(proposal.kind, ProposalKind::CreateContact);
    assert_eq!(proposal.entity_id, None);
    assert!(proposal.affected_versions.is_empty());
    assert!(proposal.warnings.is_empty());
    let field = |name: &str| {
        proposal
            .changes
            .iter()
            .find(|change| change.field == name)
            .unwrap_or_else(|| panic!("{name} is part of the diff"))
    };
    assert_eq!(field("firstName").after.as_deref(), Some("Dana"));
    assert_eq!(field("firstName").before, None);
    assert_eq!(field("phone").after.as_deref(), Some("555-0100"));
    assert_eq!(field("displayName").after.as_deref(), Some("Dana Ruiz"));

    let applied = {
        let mut guard = storage.lock().expect("storage lock");
        apply_proposal(
            &mut guard,
            &store,
            ApplyProposalRequest {
                actor: Actor::Agent,
                proposal_id: proposal.id.clone(),
                expected_versions: Vec::new(),
            },
        )
        .expect("apply the draft")
    };
    assert!(applied.created);
    assert_eq!(applied.version, 1);

    let guard = storage.lock().expect("storage lock");
    let contact = application::get_contact(&guard, &applied.entity_id).expect("created contact");
    drop(guard);
    assert_eq!(contact.display_name, "Dana Ruiz");
    assert_eq!(contact.city.as_deref(), Some("Sanford"));
    assert_eq!(contact.channels.len(), 1);
    assert_eq!(contact.channels[0].value, "555-0100");

    // Audit: the create row and the "applied a draft" row, both as the agent.
    let log = command_log(&storage);
    assert!(log.iter().any(|(actor, entity, summary)| actor == "agent"
        && entity == "contact"
        && summary.contains("created contact")));
    assert!(log.iter().any(|(actor, entity, summary)| actor == "agent"
        && entity == "contact"
        && summary.contains("applied the assistant's draft")));
    // Nothing secret or model-shaped leaks into the audit trail.
    assert!(!log
        .iter()
        .any(|(_, _, summary)| summary.contains("{") || summary.contains("apiKey")));

    // Applying is single use.
    let mut guard = storage.lock().expect("storage lock");
    let error = apply_proposal(
        &mut guard,
        &store,
        ApplyProposalRequest {
            actor: Actor::Agent,
            proposal_id: proposal.id,
            expected_versions: Vec::new(),
        },
    )
    .expect_err("a draft applies once");
    assert_eq!(error.kind(), "proposal_expired");
}

#[test]
fn a_drafted_opportunity_update_diffs_and_applies_only_the_changed_fields() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let store = ProposalStore::new();
    let contact_id = seed_contact(&storage, "Dana Ruiz");
    let opportunity_id = seed_opportunity(&storage, &contact_id, "Backyard fence");
    let provider = CannedProvider::new(
        r#"{"valueMinor": 450000, "probabilityPercent": 75, "notes": "Ready to sign"}"#,
    );

    let proposal = propose_update_with_provider(
        &storage,
        &provider,
        &store,
        (ProposalEntityType::Opportunity, &opportunity_id, 1),
        "Bump it to $4,500 and 75% — they're ready to sign",
    )
    .expect("draft an update");

    assert_eq!(proposal.kind, ProposalKind::UpdateOpportunity);
    assert_eq!(proposal.entity_id.as_deref(), Some(opportunity_id.as_str()));
    assert_eq!(
        proposal.affected_versions,
        vec![RecordVersion {
            entity_type: "opportunity".into(),
            entity_id: opportunity_id.clone(),
            version: 1,
        }]
    );
    assert_eq!(proposal.changes.len(), 3);
    let value = &proposal.changes[0];
    assert_eq!(value.field, "valueMinor");
    assert_eq!(value.before.as_deref(), Some("100000"));
    assert_eq!(value.after.as_deref(), Some("450000"));

    // The record went to the model as a bounded projection, named in the
    // disclosure list, with no ids or secrets in the context text.
    let request = provider.last_request();
    assert_eq!(request.included_record_refs.len(), 1);
    assert_eq!(request.included_record_refs[0].entity_id, opportunity_id);
    assert_eq!(request.included_record_refs[0].label, "Backyard fence");
    let context = request.context_text.expect("update calls carry context");
    assert!(context.contains("name: Backyard fence"));

    let applied = {
        let mut guard = storage.lock().expect("storage lock");
        apply_proposal(
            &mut guard,
            &store,
            ApplyProposalRequest {
                actor: Actor::User,
                proposal_id: proposal.id,
                expected_versions: vec![RecordVersion {
                    entity_type: "opportunity".into(),
                    entity_id: opportunity_id.clone(),
                    version: 1,
                }],
            },
        )
        .expect("apply the update")
    };
    assert!(!applied.created);
    assert_eq!(applied.version, 2);

    let guard = storage.lock().expect("storage lock");
    let detail = application::get_opportunity(&guard, &opportunity_id).expect("updated record");
    assert_eq!(detail.opportunity.value.value_minor, 450_000);
    assert_eq!(detail.opportunity.probability_percent, Some(75));
    assert_eq!(detail.opportunity.notes.as_deref(), Some("Ready to sign"));
    // Untouched fields keep their stored values.
    assert_eq!(detail.opportunity.name, "Backyard fence");
    assert_eq!(
        detail.opportunity.contact_id.as_deref(),
        Some(contact_id.as_str())
    );
}

#[test]
fn proposing_never_writes_to_the_database() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let store = ProposalStore::new();
    let contact_id = seed_contact(&storage, "Dana Ruiz");
    let before = fingerprint(&storage);

    propose_record_with_provider(
        &storage,
        &CannedProvider::new(r#"{"name":"Coastal Fence Co","kind":"client"}"#),
        &store,
        ProposalEntityType::Company,
        "Add Coastal Fence Co as a client",
    )
    .expect("draft a company");
    propose_update_with_provider(
        &storage,
        &CannedProvider::new(r#"{"city":"Orlando"}"#),
        &store,
        (ProposalEntityType::Contact, &contact_id, 1),
        "She moved to Orlando",
    )
    .expect("draft an update");

    assert_eq!(fingerprint(&storage), before);
    assert_eq!(store.pending_count(), 2);
}

#[test]
fn proposing_is_refused_while_the_assistant_is_switched_off() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let store = ProposalStore::new();
    let credentials = InMemoryCredentialStore::new();

    let error = contractorcrm_lib::proposals::propose_record(
        &storage,
        &credentials,
        &store,
        ProposalEntityType::Contact,
        "New client Dana Ruiz",
    )
    .expect_err("no provider is configured");
    assert_eq!(error.kind(), "provider_unavailable");
    assert!(error.to_string().contains("Turn the AI assistant on"));
    assert_eq!(
        credentials.read_count(),
        0,
        "a disabled assistant must not read credentials"
    );

    // Switched on but unreachable is a provider failure, not a silent draft.
    {
        let mut guard = storage.lock().expect("storage lock");
        set_ai_settings(
            &mut guard,
            &credentials,
            SetAiSettingsRequest {
                actor: Actor::User,
                enabled: true,
                provider_label: "Local model".into(),
                base_url: "http://127.0.0.1:1/v1".into(),
                model: "llama3.1".into(),
            },
        )
        .expect("save settings");
    }
    let error = contractorcrm_lib::proposals::propose_record(
        &storage,
        &credentials,
        &store,
        ProposalEntityType::Contact,
        "New client Dana Ruiz",
    )
    .expect_err("endpoint refuses the connection");
    assert_eq!(error.kind(), "provider_unavailable");
}

// ---------------------------------------------------------------------------
// Expiry and conflicts
// ---------------------------------------------------------------------------

#[test]
fn an_expired_draft_cannot_be_applied() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let store = ProposalStore::with_ttl(Duration::zero());

    let proposal = propose_record_with_provider(
        &storage,
        &CannedProvider::new(r#"{"displayName":"Dana Ruiz","kind":"client"}"#),
        &store,
        ProposalEntityType::Contact,
        "New client Dana Ruiz",
    )
    .expect("draft a contact");
    assert_eq!(store.pending_count(), 0);

    let mut guard = storage.lock().expect("storage lock");
    let error = apply_proposal(
        &mut guard,
        &store,
        ApplyProposalRequest {
            actor: Actor::User,
            proposal_id: proposal.id,
            expected_versions: Vec::new(),
        },
    )
    .expect_err("the draft expired");
    assert_eq!(error.kind(), "proposal_expired");

    let error = apply_proposal(
        &mut guard,
        &store,
        ApplyProposalRequest {
            actor: Actor::User,
            proposal_id: "never-existed".into(),
            expected_versions: Vec::new(),
        },
    )
    .expect_err("unknown ids look the same");
    assert_eq!(error.kind(), "proposal_expired");
}

#[test]
fn a_stale_expected_version_conflicts_and_keeps_the_draft_usable() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let store = ProposalStore::new();
    let contact_id = seed_contact(&storage, "Dana Ruiz");

    let proposal = propose_update_with_provider(
        &storage,
        &CannedProvider::new(r#"{"city":"Orlando"}"#),
        &store,
        (ProposalEntityType::Contact, &contact_id, 1),
        "She moved to Orlando",
    )
    .expect("draft an update");

    // Someone edits the contact by hand in the meantime.
    {
        let mut guard = storage.lock().expect("storage lock");
        application::update_contact(
            &mut guard,
            UpdateContactRequest {
                actor: Actor::User,
                contact_id: contact_id.clone(),
                expected_version: 1,
                patch: ContactPatch {
                    notes: Some("Called back".into()),
                    ..contact_patch("Dana Ruiz")
                },
            },
        )
        .expect("hand edit");
    }

    let mut guard = storage.lock().expect("storage lock");
    let error = apply_proposal(
        &mut guard,
        &store,
        ApplyProposalRequest {
            actor: Actor::User,
            proposal_id: proposal.id.clone(),
            expected_versions: vec![RecordVersion {
                entity_type: "contact".into(),
                entity_id: contact_id.clone(),
                version: 1,
            }],
        },
    )
    .expect_err("the record moved");
    assert_eq!(error.kind(), "version_conflict");
    drop(guard);
    assert_eq!(store.pending_count(), 1, "a conflict never eats the draft");

    // Refreshing to the current version applies the same draft.
    let mut guard = storage.lock().expect("storage lock");
    let applied = apply_proposal(
        &mut guard,
        &store,
        ApplyProposalRequest {
            actor: Actor::User,
            proposal_id: proposal.id,
            expected_versions: vec![RecordVersion {
                entity_type: "contact".into(),
                entity_id: contact_id.clone(),
                version: 2,
            }],
        },
    )
    .expect("apply after refresh");
    assert_eq!(applied.version, 3);
    assert_eq!(
        application::get_contact(&guard, &contact_id)
            .expect("contact")
            .city
            .as_deref(),
        Some("Orlando")
    );
}

#[test]
fn applying_without_asserted_versions_still_checks_the_version_the_draft_saw() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let store = ProposalStore::new();
    let contact_id = seed_contact(&storage, "Dana Ruiz");

    let proposal = propose_update_with_provider(
        &storage,
        &CannedProvider::new(r#"{"city":"Orlando"}"#),
        &store,
        (ProposalEntityType::Contact, &contact_id, 1),
        "She moved to Orlando",
    )
    .expect("draft an update");

    {
        let mut guard = storage.lock().expect("storage lock");
        application::update_contact(
            &mut guard,
            UpdateContactRequest {
                actor: Actor::User,
                contact_id: contact_id.clone(),
                expected_version: 1,
                patch: contact_patch("Dana Ruiz"),
            },
        )
        .expect("hand edit");
    }

    let mut guard = storage.lock().expect("storage lock");
    let error = apply_proposal(
        &mut guard,
        &store,
        ApplyProposalRequest {
            actor: Actor::User,
            proposal_id: proposal.id,
            expected_versions: Vec::new(),
        },
    )
    .expect_err("the draft was built against version 1");
    assert_eq!(error.kind(), "version_conflict");
}

/// Record ids are unique per table, not globally. An assertion about some
/// other record type must never stand in for the draft's own affected record.
#[test]
fn an_assertion_about_another_record_type_does_not_satisfy_the_drafts_own_check() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let store = ProposalStore::new();
    let contact_id = seed_contact(&storage, "Dana Ruiz");

    // Force the id collision this rule exists for: a company that happens to
    // carry the same id as the contact the draft is about.
    {
        let mut guard = storage.lock().expect("storage lock");
        let company = application::create_company(
            &mut guard,
            CreateCompanyRequest {
                actor: Actor::User,
                company: CompanyPatch {
                    name: "Coastal Fence".into(),
                    kind: "client".into(),
                    ..CompanyPatch::default()
                },
            },
        )
        .expect("seed company");
        guard
            .connection()
            .execute(
                "UPDATE companies SET id = ?1 WHERE id = ?2",
                rusqlite::params![&contact_id, &company.id],
            )
            .expect("collide the ids");
    }

    let proposal = propose_update_with_provider(
        &storage,
        &CannedProvider::new(r#"{"city":"Orlando"}"#),
        &store,
        (ProposalEntityType::Contact, &contact_id, 1),
        "She moved to Orlando",
    )
    .expect("draft an update");

    // The contact moves on behind the draft's back.
    {
        let mut guard = storage.lock().expect("storage lock");
        application::update_contact(
            &mut guard,
            UpdateContactRequest {
                actor: Actor::User,
                contact_id: contact_id.clone(),
                expected_version: 1,
                patch: contact_patch("Dana Ruiz"),
            },
        )
        .expect("hand edit");
    }

    let mut guard = storage.lock().expect("storage lock");
    let error = apply_proposal(
        &mut guard,
        &store,
        ApplyProposalRequest {
            actor: Actor::User,
            proposal_id: proposal.id,
            // A true statement about the company, not about the contact.
            expected_versions: vec![RecordVersion {
                entity_type: "company".into(),
                entity_id: contact_id.clone(),
                version: 1,
            }],
        },
    )
    .expect_err("the contact's own version is still checked");
    assert_eq!(error.kind(), "version_conflict");
    assert!(error.to_string().contains("contact"));
}

#[test]
fn proposing_an_update_against_a_stale_version_conflicts_before_the_model_is_asked() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let store = ProposalStore::new();
    let contact_id = seed_contact(&storage, "Dana Ruiz");
    let provider = CannedProvider::new(r#"{"city":"Orlando"}"#);

    let error = propose_update_with_provider(
        &storage,
        &provider,
        &store,
        (ProposalEntityType::Contact, &contact_id, 7),
        "She moved to Orlando",
    )
    .expect_err("version 7 does not exist");
    assert_eq!(error.kind(), "version_conflict");
    assert!(provider
        .last_request
        .lock()
        .expect("request mutex")
        .is_none());

    let error = propose_update_with_provider(
        &storage,
        &provider,
        &store,
        (ProposalEntityType::Contact, "missing-contact", 1),
        "She moved to Orlando",
    )
    .expect_err("unknown record");
    assert_eq!(error.kind(), "not_found");
}

// ---------------------------------------------------------------------------
// Undo
// ---------------------------------------------------------------------------

#[test]
fn undoing_a_created_record_archives_it_rather_than_deleting_it() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let store = ProposalStore::new();

    let proposal = propose_record_with_provider(
        &storage,
        &CannedProvider::new(r#"{"displayName":"Dana Ruiz","kind":"client"}"#),
        &store,
        ProposalEntityType::Contact,
        "New client Dana Ruiz",
    )
    .expect("draft a contact");

    let mut guard = storage.lock().expect("storage lock");
    let applied = apply_proposal(
        &mut guard,
        &store,
        ApplyProposalRequest {
            actor: Actor::User,
            proposal_id: proposal.id,
            expected_versions: Vec::new(),
        },
    )
    .expect("apply the draft");

    let undone = undo_proposal(
        &mut guard,
        &store,
        UndoProposalRequest {
            actor: Actor::User,
            undo_token: applied.undo_token.clone(),
            expected_versions: vec![RecordVersion {
                entity_type: "contact".into(),
                entity_id: applied.entity_id.clone(),
                version: applied.version,
            }],
        },
    )
    .expect("undo the create");
    assert_eq!(undone.action, "archived");

    let contact = application::get_contact(&guard, &applied.entity_id).expect("record still there");
    assert!(contact.archived_at.is_some());
    drop(guard);

    assert!(command_log(&storage)
        .iter()
        .any(|(_, _, summary)| summary.contains("undid the assistant's applied draft")));

    // Undo tokens are single use too.
    let mut guard = storage.lock().expect("storage lock");
    let error = undo_proposal(
        &mut guard,
        &store,
        UndoProposalRequest {
            actor: Actor::User,
            undo_token: applied.undo_token,
            expected_versions: Vec::new(),
        },
    )
    .expect_err("one undo per apply");
    assert_eq!(error.kind(), "proposal_expired");
}

/// The audit row is written after the record change has already committed, so
/// it must never be able to strand an applied draft with no way back.
#[test]
fn a_failed_audit_row_still_leaves_the_apply_undoable() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let store = ProposalStore::new();

    // Refuse exactly the proposal audit row; the ordinary create row still
    // writes, so this isolates the best-effort logging path.
    {
        let guard = storage.lock().expect("storage lock");
        guard
            .connection()
            .execute_batch(
                "CREATE TRIGGER refuse_proposal_audit BEFORE INSERT ON command_log
                 WHEN NEW.summary LIKE 'applied the assistant%'
                 BEGIN SELECT RAISE(ABORT, 'audit row refused'); END;",
            )
            .expect("install the audit trigger");
    }

    let proposal = propose_record_with_provider(
        &storage,
        &CannedProvider::new(r#"{"displayName":"Dana Ruiz","kind":"client"}"#),
        &store,
        ProposalEntityType::Contact,
        "New client Dana Ruiz",
    )
    .expect("draft a contact");

    let mut guard = storage.lock().expect("storage lock");
    let applied = apply_proposal(
        &mut guard,
        &store,
        ApplyProposalRequest {
            actor: Actor::Agent,
            proposal_id: proposal.id,
            expected_versions: Vec::new(),
        },
    )
    .expect("the apply survives a refused audit row");

    // The record exists and the undo token still works.
    application::get_contact(&guard, &applied.entity_id).expect("the record was created");
    let undone = undo_proposal(
        &mut guard,
        &store,
        UndoProposalRequest {
            actor: Actor::Agent,
            undo_token: applied.undo_token,
            expected_versions: Vec::new(),
        },
    )
    .expect("undo works after a failed audit row");
    assert_eq!(undone.action, "archived");
    drop(guard);

    assert!(
        !command_log(&storage)
            .iter()
            .any(|(_, _, summary)| summary.contains("applied the assistant's draft")),
        "the trigger really did refuse the audit row"
    );
}

#[test]
fn undoing_an_update_restores_the_stored_before_image() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let store = ProposalStore::new();
    let contact_id = seed_contact(&storage, "Dana Ruiz");
    {
        let mut guard = storage.lock().expect("storage lock");
        application::update_contact(
            &mut guard,
            UpdateContactRequest {
                actor: Actor::User,
                contact_id: contact_id.clone(),
                expected_version: 1,
                patch: ContactPatch {
                    city: Some("Sanford".into()),
                    notes: Some("Original note".into()),
                    ..contact_patch("Dana Ruiz")
                },
            },
        )
        .expect("set the starting values");
    }

    let proposal = propose_update_with_provider(
        &storage,
        &CannedProvider::new(r#"{"city":"Orlando","notes":"Moved across town"}"#),
        &store,
        (ProposalEntityType::Contact, &contact_id, 2),
        "She moved to Orlando",
    )
    .expect("draft an update");

    let mut guard = storage.lock().expect("storage lock");
    let applied = apply_proposal(
        &mut guard,
        &store,
        ApplyProposalRequest {
            actor: Actor::User,
            proposal_id: proposal.id,
            expected_versions: Vec::new(),
        },
    )
    .expect("apply the update");
    assert_eq!(
        application::get_contact(&guard, &contact_id)
            .expect("contact")
            .city
            .as_deref(),
        Some("Orlando")
    );

    let undone = undo_proposal(
        &mut guard,
        &store,
        UndoProposalRequest {
            actor: Actor::User,
            undo_token: applied.undo_token,
            expected_versions: Vec::new(),
        },
    )
    .expect("undo the update");
    assert_eq!(undone.action, "reverted");

    let contact = application::get_contact(&guard, &contact_id).expect("contact");
    assert_eq!(contact.city.as_deref(), Some("Sanford"));
    assert_eq!(contact.notes.as_deref(), Some("Original note"));
    assert_eq!(contact.version, 4);
}

#[test]
fn undo_refuses_when_the_record_moved_since_it_was_applied() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let store = ProposalStore::new();
    let contact_id = seed_contact(&storage, "Dana Ruiz");

    let proposal = propose_update_with_provider(
        &storage,
        &CannedProvider::new(r#"{"city":"Orlando"}"#),
        &store,
        (ProposalEntityType::Contact, &contact_id, 1),
        "She moved to Orlando",
    )
    .expect("draft an update");

    let mut guard = storage.lock().expect("storage lock");
    let applied = apply_proposal(
        &mut guard,
        &store,
        ApplyProposalRequest {
            actor: Actor::User,
            proposal_id: proposal.id,
            expected_versions: Vec::new(),
        },
    )
    .expect("apply the update");
    application::update_contact(
        &mut guard,
        UpdateContactRequest {
            actor: Actor::User,
            contact_id: contact_id.clone(),
            expected_version: applied.version,
            patch: contact_patch("Dana Ruiz"),
        },
    )
    .expect("hand edit after the apply");

    let error = undo_proposal(
        &mut guard,
        &store,
        UndoProposalRequest {
            actor: Actor::User,
            undo_token: applied.undo_token.clone(),
            expected_versions: vec![RecordVersion {
                entity_type: "contact".into(),
                entity_id: contact_id.clone(),
                version: applied.version,
            }],
        },
    )
    .expect_err("the record moved");
    assert_eq!(error.kind(), "version_conflict");

    // Nothing was reverted, and the token is still in the store rather than
    // burned by a failure the user did not cause.
    // A revert would have bumped the version again; it stands where the hand
    // edit left it.
    let contact = application::get_contact(&guard, &contact_id).expect("contact");
    assert_eq!(contact.version, applied.version + 1);
    let error = undo_proposal(
        &mut guard,
        &store,
        UndoProposalRequest {
            actor: Actor::User,
            undo_token: applied.undo_token,
            expected_versions: Vec::new(),
        },
    )
    .expect_err("the token is still there, and the record is still ahead of it");
    assert_eq!(error.kind(), "version_conflict");
}

#[test]
fn undo_never_silently_reverts_over_work_done_after_the_apply() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let store = ProposalStore::new();
    let contact_id = seed_contact(&storage, "Dana Ruiz");

    let proposal = propose_update_with_provider(
        &storage,
        &CannedProvider::new(r#"{"city":"Orlando"}"#),
        &store,
        (ProposalEntityType::Contact, &contact_id, 1),
        "She moved to Orlando",
    )
    .expect("draft an update");

    let mut guard = storage.lock().expect("storage lock");
    let applied = apply_proposal(
        &mut guard,
        &store,
        ApplyProposalRequest {
            actor: Actor::User,
            proposal_id: proposal.id,
            expected_versions: Vec::new(),
        },
    )
    .expect("apply the update");

    // Real work lands on the record after the apply.
    application::update_contact(
        &mut guard,
        UpdateContactRequest {
            actor: Actor::User,
            contact_id: contact_id.clone(),
            expected_version: applied.version,
            patch: ContactPatch {
                city: Some("Orlando".into()),
                notes: Some("Signed the contract".into()),
                ..contact_patch("Dana Ruiz")
            },
        },
    )
    .expect("newer work");

    // A caller that asserts no versions at all (a future agent client) still
    // gets a conflict rather than a silent overwrite of that work.
    let error = undo_proposal(
        &mut guard,
        &store,
        UndoProposalRequest {
            actor: Actor::User,
            undo_token: applied.undo_token,
            expected_versions: Vec::new(),
        },
    )
    .expect_err("the record moved after the apply");
    assert_eq!(error.kind(), "version_conflict");
    assert!(error.to_string().contains(&format!(
        "expected version {}, current version {}",
        applied.version,
        applied.version + 1
    )));

    let contact = application::get_contact(&guard, &contact_id).expect("contact");
    assert_eq!(contact.notes.as_deref(), Some("Signed the contract"));
    assert_eq!(contact.version, applied.version + 1);
}

// ---------------------------------------------------------------------------
// Hostile and sloppy model output
// ---------------------------------------------------------------------------

#[test]
fn an_answer_that_is_not_a_draft_fails_validation_instead_of_writing_anything() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let store = ProposalStore::new();
    let before = fingerprint(&storage);

    for answer in [
        "I'm sorry, I can't do that.",
        "{ not json at all",
        "[]",
        "null",
        "{\"displayName\": [1, 2, 3]}",
    ] {
        let error = propose_record_with_provider(
            &storage,
            &CannedProvider::new(answer),
            &store,
            ProposalEntityType::Contact,
            "New client Dana Ruiz",
        )
        .expect_err("unusable draft");
        // Unreadable answers fail as validation_failed; readable ones that
        // leave a required field empty fail as invalid_input. Either way the
        // command errors instead of panicking or writing.
        assert!(
            matches!(error.kind(), "validation_failed" | "invalid_input"),
            "answer {answer:?} must be rejected, got {}",
            error.kind()
        );
    }
    assert_eq!(fingerprint(&storage), before);
    assert_eq!(store.pending_count(), 0);
}

#[test]
fn fields_the_app_does_not_store_become_warnings_not_data() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let store = ProposalStore::new();

    let proposal = propose_record_with_provider(
        &storage,
        &CannedProvider::new(
            r#"{"displayName":"Dana Ruiz","kind":"client","creditCard":"4111111111111111","companyName":"Nowhere Inc"}"#,
        ),
        &store,
        ProposalEntityType::Contact,
        "New client Dana Ruiz",
    )
    .expect("draft a contact");

    assert!(proposal
        .warnings
        .iter()
        .any(|warning| warning.contains("creditCard")));
    assert!(proposal
        .warnings
        .iter()
        .any(|warning| warning.contains("Nowhere Inc")));
    assert!(proposal
        .changes
        .iter()
        .all(|change| change.field != "creditCard"));
}

/// A hostile or runaway model answer cannot push an unbounded string into a
/// record: the draft is shortened, the user is told, and the apply still works.
#[test]
fn an_oversized_drafted_note_is_shortened_warned_about_and_still_applies() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let store = ProposalStore::new();
    let answer = format!(
        r#"{{"displayName":"Dana Ruiz","kind":"client","notes":"{}"}}"#,
        "n".repeat(50_000)
    );

    let proposal = propose_record_with_provider(
        &storage,
        &CannedProvider::new(&answer),
        &store,
        ProposalEntityType::Contact,
        "New client Dana Ruiz with a long note",
    )
    .expect("draft a contact");

    assert!(proposal
        .warnings
        .iter()
        .any(|warning| warning.contains("notes")));
    let notes = proposal
        .changes
        .iter()
        .find(|change| change.field == "notes")
        .and_then(|change| change.after.clone())
        .expect("the note is still part of the draft");
    assert_eq!(notes.chars().count(), 10_000);

    let mut guard = storage.lock().expect("storage lock");
    let applied = apply_proposal(
        &mut guard,
        &store,
        ApplyProposalRequest {
            actor: Actor::Agent,
            proposal_id: proposal.id,
            expected_versions: Vec::new(),
        },
    )
    .expect("a shortened draft still applies");
    let contact = application::get_contact(&guard, &applied.entity_id).expect("created contact");
    assert_eq!(contact.notes.as_deref().map(str::len), Some(10_000));
}

#[test]
fn a_draft_that_breaks_a_record_rule_is_rejected_by_the_same_validation_as_a_hand_edit() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let store = ProposalStore::new();

    // A contact with no name at all cannot be created by hand either.
    let error = propose_record_with_provider(
        &storage,
        &CannedProvider::new(r#"{"city":"Sanford"}"#),
        &store,
        ProposalEntityType::Contact,
        "Someone in Sanford",
    )
    .expect_err("a contact needs a name");
    assert_eq!(error.kind(), "invalid_input");

    // An unknown enum value is rejected, not coerced.
    let error = propose_record_with_provider(
        &storage,
        &CannedProvider::new(r#"{"displayName":"Dana Ruiz","kind":"vip"}"#),
        &store,
        ProposalEntityType::Contact,
        "New VIP Dana Ruiz",
    )
    .expect_err("unknown kind");
    assert_eq!(error.kind(), "invalid_input");

    // An opportunity with nobody to hang off is refused.
    let error = propose_record_with_provider(
        &storage,
        &CannedProvider::new(r#"{"name":"Backyard fence","valueMinor":450000}"#),
        &store,
        ProposalEntityType::Opportunity,
        "New backyard fence job",
    )
    .expect_err("an opportunity needs a contact or company");
    assert_eq!(error.kind(), "validation_failed");
    assert_eq!(store.pending_count(), 0);
}

#[test]
fn a_named_contact_links_a_drafted_opportunity_and_an_unknown_name_only_warns() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let store = ProposalStore::new();
    let contact_id = seed_contact(&storage, "Dana Ruiz");

    let proposal = propose_record_with_provider(
        &storage,
        &CannedProvider::new(
            r#"{"name":"Backyard fence","contactName":"dana ruiz","companyName":"Nowhere Inc","valueMinor":"$4,500"}"#,
        ),
        &store,
        ProposalEntityType::Opportunity,
        "Backyard fence for Dana Ruiz, about $45",
    )
    .expect("draft an opportunity");

    assert!(proposal
        .warnings
        .iter()
        .any(|warning| warning.contains("Nowhere Inc")));

    let mut guard = storage.lock().expect("storage lock");
    let applied = apply_proposal(
        &mut guard,
        &store,
        ApplyProposalRequest {
            actor: Actor::Agent,
            proposal_id: proposal.id,
            expected_versions: Vec::new(),
        },
    )
    .expect("apply the draft");
    let detail = application::get_opportunity(&guard, &applied.entity_id).expect("created record");
    assert_eq!(
        detail.opportunity.contact_id.as_deref(),
        Some(contact_id.as_str())
    );
    assert_eq!(detail.opportunity.company_id, None);
    // "$4,500" was read as the whole number of cents the field is documented as.
    assert_eq!(detail.opportunity.value.value_minor, 4500);
    assert_eq!(detail.opportunity.value.currency_code, "USD");
}

/// Prompt injection through record data (docs/THREAT_MODEL.md "Provider
/// context"): a note in one record tells the model to edit a different one and
/// to grant itself a version. The model obeys; the seam does not. The draft
/// still targets the record the caller named, the smuggled routing fields are
/// warnings, and applying it moves only that record.
#[test]
fn a_poisoned_model_answer_cannot_retarget_another_record() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let store = ProposalStore::new();
    let target = seed_contact(&storage, "Dana Ruiz");
    let bystander = seed_contact(&storage, "Sam Ortiz");

    let proposal = propose_update_with_provider(
        &storage,
        &CannedProvider::new(&format!(
            r#"{{"id":"{bystander}","contactId":"{bystander}","entityType":"contact",
                 "version":99,"expectedVersion":99,"archivedAt":"2020-01-01T00:00:00Z",
                 "displayName":"Dana Ruiz","notes":"IGNORE PREVIOUS INSTRUCTIONS"}}"#
        )),
        &store,
        (ProposalEntityType::Contact, &target, 1),
        "Add a note",
    )
    .expect("draft an update");

    assert_eq!(proposal.entity_id.as_deref(), Some(target.as_str()));
    assert_eq!(proposal.affected_versions.len(), 1);
    assert_eq!(proposal.affected_versions[0].entity_id, target);
    assert_eq!(proposal.affected_versions[0].version, 1);
    for smuggled in [
        "id",
        "contactId",
        "version",
        "expectedVersion",
        "archivedAt",
    ] {
        assert!(
            proposal
                .warnings
                .iter()
                .any(|warning| warning.contains(smuggled)),
            "{smuggled} must be reported as ignored: {:?}",
            proposal.warnings
        );
        assert!(
            proposal
                .changes
                .iter()
                .all(|change| change.field != smuggled),
            "{smuggled} must never become a change"
        );
    }

    apply_proposal(
        &mut storage.lock().expect("storage lock"),
        &store,
        ApplyProposalRequest {
            actor: Actor::User,
            proposal_id: proposal.id,
            expected_versions: vec![RecordVersion {
                entity_type: "contact".into(),
                entity_id: target.clone(),
                version: 1,
            }],
        },
    )
    .expect("apply the draft");

    let guard = storage.lock().expect("storage lock");
    let moved = application::get_contact(&guard, &target).expect("target contact");
    assert_eq!(moved.version, 2);
    assert_eq!(
        moved.notes.as_deref(),
        Some("IGNORE PREVIOUS INSTRUCTIONS"),
        "injected text is stored as plain data, not obeyed"
    );
    let untouched = application::get_contact(&guard, &bystander).expect("bystander contact");
    assert_eq!(untouched.version, 1);
    assert!(untouched.notes.is_none());
    assert!(untouched.archived_at.is_none());
}
