//! Integration tests for follow-up templates, history summaries, and follow-up
//! drafting: the stored template contract, the bounded timeline projection, the
//! assistant-off invariants, and the proposal → apply → undo path for a drafted
//! follow-up task. No network and no real keychain — the provider is canned and
//! credentials live in memory.

use std::sync::Mutex;

use chrono::{Duration, SecondsFormat, Utc};
use contractorcrm_lib::ai::{
    set_ai_settings, CompletionProvider, InMemoryCredentialStore, ProviderCheck,
    ProviderCompletion, ProviderRequest, SetAiSettingsRequest,
};
use contractorcrm_lib::application::{
    self, ActivityPatch, ContactPatch, CreateContactRequest, ListTasksRequest, LogActivityRequest,
};
use contractorcrm_lib::domain::{Actor, TaskStatus};
use contractorcrm_lib::error::ApplicationError;
use contractorcrm_lib::followups::{
    default_templates, get_followup_templates, plan_history_summary, propose_followup,
    propose_followup_with, set_followup_templates, FollowupTemplate, SetFollowupTemplatesRequest,
    FOLLOWUP_TEMPLATES_VERSION,
};
use contractorcrm_lib::proposals::{
    apply_proposal, undo_proposal, ApplyProposalRequest, ProposalEntityType, ProposalKind,
    ProposalStore, RecordVersion, UndoProposalRequest,
};
use contractorcrm_lib::storage::Storage;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// A provider that answers with canned text and records what it was asked.
struct CannedProvider {
    text: String,
    seen: Mutex<Vec<ProviderRequest>>,
}

impl CannedProvider {
    fn new(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            seen: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<ProviderRequest> {
        self.seen.lock().expect("canned provider mutex").clone()
    }
}

impl CompletionProvider for CannedProvider {
    fn label(&self) -> &str {
        "Canned model"
    }

    fn complete(&self, request: &ProviderRequest) -> Result<ProviderCompletion, ApplicationError> {
        self.seen
            .lock()
            .expect("canned provider mutex")
            .push(request.clone());
        Ok(ProviderCompletion {
            purpose: request.purpose.clone(),
            model: "canned-model".into(),
            text: self.text.clone(),
            included_record_refs: request.included_record_refs.clone(),
        })
    }

    fn check(&self) -> Result<ProviderCheck, ApplicationError> {
        unreachable!("follow-ups never run a connection check")
    }
}

fn open_storage(temp: &tempfile::TempDir) -> Mutex<Storage> {
    Mutex::new(Storage::open_in_app_data(temp.path()).expect("open storage"))
}

fn days_ago(days: i64) -> String {
    (Utc::now() - Duration::days(days)).to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn seed_contact(storage: &Mutex<Storage>, display_name: &str) -> String {
    let mut guard = storage.lock().expect("storage lock");
    application::create_contact(
        &mut guard,
        CreateContactRequest {
            actor: Actor::User,
            contact: ContactPatch {
                display_name: Some(display_name.into()),
                kind: "lead".into(),
                ..ContactPatch::default()
            },
        },
    )
    .expect("seed contact")
    .id
}

fn log_touch(
    storage: &Mutex<Storage>,
    contact_id: &str,
    summary: &str,
    body: Option<&str>,
    occurred_at: &str,
) {
    let mut guard = storage.lock().expect("storage lock");
    application::log_activity(
        &mut guard,
        LogActivityRequest {
            actor: Actor::User,
            parent_type: "contact".into(),
            parent_id: contact_id.into(),
            activity: ActivityPatch {
                kind: "call".into(),
                direction: Some("outbound".into()),
                occurred_at: Some(occurred_at.into()),
                summary: summary.into(),
                body: body.map(str::to_owned),
            },
        },
    )
    .expect("log activity");
}

fn turn_the_assistant_on(storage: &Mutex<Storage>, credentials: &InMemoryCredentialStore) {
    let mut guard = storage.lock().expect("storage lock");
    set_ai_settings(
        &mut guard,
        credentials,
        SetAiSettingsRequest {
            actor: Actor::User,
            enabled: true,
            provider_label: "Local model".into(),
            base_url: "http://127.0.0.1:11434/v1".into(),
            model: "llama3.1".into(),
        },
    )
    .expect("enable the assistant");
}

// ---------------------------------------------------------------------------
// Templates
// ---------------------------------------------------------------------------

#[test]
fn the_built_in_templates_are_returned_until_they_are_edited() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let mut guard = storage.lock().expect("storage lock");

    let stored = get_followup_templates(&guard).expect("read defaults");
    assert_eq!(stored.version, FOLLOWUP_TEMPLATES_VERSION);
    assert_eq!(stored.templates, default_templates());
    assert!(stored
        .templates
        .iter()
        .all(|template| !template.body.trim().is_empty()));

    let saved = set_followup_templates(
        &mut guard,
        SetFollowupTemplatesRequest {
            actor: Actor::User,
            templates: vec![FollowupTemplate {
                id: "gate_check".into(),
                name: "Gate check-in".into(),
                body: "Checking in on the gate opener.".into(),
            }],
        },
    )
    .expect("save templates");
    assert_eq!(saved.templates.len(), 1);
    assert_eq!(
        get_followup_templates(&guard).expect("read back").templates,
        saved.templates
    );

    // Saving an empty set restores the built-ins rather than leaving nothing.
    let restored = set_followup_templates(
        &mut guard,
        SetFollowupTemplatesRequest {
            actor: Actor::User,
            templates: Vec::new(),
        },
    )
    .expect("empty set restores defaults");
    assert_eq!(restored.templates, default_templates());
}

#[test]
fn template_writes_are_validated_and_capped() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let mut guard = storage.lock().expect("storage lock");

    for templates in [
        vec![FollowupTemplate {
            id: "blank".into(),
            name: " ".into(),
            body: "Body".into(),
        }],
        vec![FollowupTemplate {
            id: "long".into(),
            name: "Name".into(),
            body: "x".repeat(2001),
        }],
        (0..21)
            .map(|index| FollowupTemplate {
                id: format!("t{index}"),
                name: format!("Template {index}"),
                body: "Body".into(),
            })
            .collect::<Vec<_>>(),
    ] {
        let error = set_followup_templates(
            &mut guard,
            SetFollowupTemplatesRequest {
                actor: Actor::User,
                templates,
            },
        )
        .expect_err("rejected");
        assert_eq!(error.kind(), "invalid_input");
    }

    // Nothing was written by any of the rejected calls.
    assert_eq!(
        get_followup_templates(&guard).expect("read").templates,
        default_templates()
    );
}

// ---------------------------------------------------------------------------
// summarize_history
// ---------------------------------------------------------------------------

#[test]
fn the_summary_projection_is_bounded_and_carries_only_the_target_record() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let credentials = InMemoryCredentialStore::new();
    turn_the_assistant_on(&storage, &credentials);

    let contact_id = seed_contact(&storage, "Dana Ruiz");
    let other_id = seed_contact(&storage, "Marco Silva");
    log_touch(
        &storage,
        &other_id,
        "Marco's private call",
        None,
        &days_ago(1),
    );
    // Thirty entries inside the window, one long body, and one outside it.
    for index in 0..30 {
        log_touch(
            &storage,
            &contact_id,
            &format!("Call {index}"),
            Some(&"x".repeat(900)),
            &days_ago(index + 1),
        );
    }
    log_touch(
        &storage,
        &contact_id,
        "Ancient history",
        None,
        &days_ago(400),
    );

    let plan = {
        let guard = storage.lock().expect("storage lock");
        plan_history_summary(&guard, &credentials, "contact", &contact_id, None)
            .expect("plan the summary")
    };

    let request = plan.request();
    assert_eq!(request.purpose, "summarize_history");
    let context = request.context_text.clone().expect("bounded projection");
    // Entry count is capped and long bodies are truncated.
    assert_eq!(plan.entry_count(), 25);
    assert_eq!(context.matches("- ").count(), 25);
    assert!(context.contains('…'));
    assert!(!context.contains(&"x".repeat(300)));
    // The window keeps older history out, and other records never appear.
    assert!(!context.contains("Ancient history"));
    assert!(!context.contains("Marco"));
    assert!(!context.contains(&other_id));
    assert!(context.contains("Record: Dana Ruiz (contact"));
    // The disclosure list names exactly the one record that was included.
    assert_eq!(request.included_record_refs.len(), 1);
    assert_eq!(request.included_record_refs[0].entity_id, contact_id);

    let provider = CannedProvider::new(
        "Summary: Dana keeps calling about the gate.\nNext actions:\n- Send the quote\n- Call Friday",
    );
    let summary = plan.run_with(&provider).expect("summary");
    assert_eq!(summary.parent_id, contact_id);
    assert_eq!(summary.model, "canned-model");
    assert_eq!(summary.summary, "Dana keeps calling about the gate.");
    assert_eq!(
        summary.suggested_next_actions,
        ["Send the quote", "Call Friday"]
    );
    assert_eq!(summary.endpoint_host, "127.0.0.1:11434");
    assert!(summary.local);
    assert_eq!(summary.included_record_refs.len(), 1);
}

#[test]
fn a_shorter_window_keeps_older_entries_out_of_the_projection() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let credentials = InMemoryCredentialStore::new();
    turn_the_assistant_on(&storage, &credentials);

    let contact_id = seed_contact(&storage, "Dana Ruiz");
    log_touch(&storage, &contact_id, "Recent call", None, &days_ago(2));
    log_touch(&storage, &contact_id, "Older call", None, &days_ago(40));

    let guard = storage.lock().expect("storage lock");
    let context = plan_history_summary(&guard, &credentials, "contact", &contact_id, Some(7))
        .expect("plan the summary")
        .request()
        .context_text
        .clone()
        .expect("projection");
    assert!(context.contains("Recent call"));
    assert!(!context.contains("Older call"));

    // The window itself is bounded.
    assert_eq!(
        plan_history_summary(&guard, &credentials, "contact", &contact_id, Some(0))
            .expect_err("zero-day window")
            .kind(),
        "invalid_input"
    );
}

#[test]
fn summarizing_with_the_assistant_off_reads_no_credentials_and_sends_nothing() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let credentials = InMemoryCredentialStore::with_key("sk-test");
    let contact_id = seed_contact(&storage, "Dana Ruiz");

    let guard = storage.lock().expect("storage lock");
    let error = plan_history_summary(&guard, &credentials, "contact", &contact_id, None)
        .expect_err("the assistant is off");
    assert_eq!(error.kind(), "provider_unavailable");
    assert_eq!(credentials.read_count(), 0);
}

#[test]
fn summarizing_an_unknown_record_is_not_found() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let credentials = InMemoryCredentialStore::new();
    turn_the_assistant_on(&storage, &credentials);

    let guard = storage.lock().expect("storage lock");
    assert_eq!(
        plan_history_summary(&guard, &credentials, "contact", "missing", None)
            .expect_err("no such contact")
            .kind(),
        "not_found"
    );
    assert_eq!(
        plan_history_summary(&guard, &credentials, "invoice", "whatever", None)
            .expect_err("no such parent type")
            .kind(),
        "invalid_input"
    );
}

// ---------------------------------------------------------------------------
// propose_followup
// ---------------------------------------------------------------------------

fn open_task_count(storage: &Mutex<Storage>) -> usize {
    let guard = storage.lock().expect("storage lock");
    application::list_tasks(
        &guard,
        ListTasksRequest {
            status: Some("open".into()),
            ..ListTasksRequest::default()
        },
    )
    .expect("list tasks")
    .len()
}

#[test]
fn a_drafted_follow_up_writes_nothing_until_it_is_applied_and_can_be_undone() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let credentials = InMemoryCredentialStore::new();
    let store = ProposalStore::new();
    turn_the_assistant_on(&storage, &credentials);

    let contact_id = seed_contact(&storage, "Dana Ruiz");
    log_touch(
        &storage,
        &contact_id,
        "Talked about the gate",
        None,
        &days_ago(3),
    );

    let provider = CannedProvider::new("Hi Dana — following up on the gate we talked about.");
    let draft = propose_followup_with(
        &storage,
        Some(&provider),
        &store,
        "contact",
        &contact_id,
        Some("chase the proposal I sent"),
        None,
    )
    .expect("draft a follow-up");

    // The wording came from the provider, against the chosen template.
    assert!(draft.used_provider);
    assert_eq!(draft.template_id, "proposal_chaser");
    assert_eq!(
        draft.draft_text,
        "Hi Dana — following up on the gate we talked about."
    );
    assert_eq!(draft.model.as_deref(), Some("canned-model"));
    assert_eq!(draft.endpoint_host, None); // the canned provider sends nowhere
    assert_eq!(draft.included_record_refs.len(), 1);

    // The call carried the template body and the bounded history, nothing else.
    let request = provider.requests().pop().expect("one provider call");
    assert_eq!(request.purpose, "propose_followup");
    assert!(request.user_text.contains("Proposal follow-up"));
    assert!(request.user_text.contains("chase the proposal I sent"));
    assert!(request
        .context_text
        .as_deref()
        .expect("history projection")
        .contains("Talked about the gate"));

    // The proposal is a follow-up task, and nothing was written yet.
    assert_eq!(draft.proposal.kind, ProposalKind::CreateFollowupTask);
    assert_eq!(draft.proposal.entity_type, ProposalEntityType::Task);
    assert_eq!(draft.proposal.entity_id, None);
    let titled = draft
        .proposal
        .changes
        .iter()
        .find(|change| change.field == "title")
        .expect("the draft sets a title");
    assert_eq!(titled.after.as_deref(), Some("chase the proposal I sent"));
    assert_eq!(open_task_count(&storage), 0);

    // Applying is the user's separate act, through the ordinary proposal path.
    let applied = {
        let mut guard = storage.lock().expect("storage lock");
        apply_proposal(
            &mut guard,
            &store,
            ApplyProposalRequest {
                actor: Actor::User,
                proposal_id: draft.proposal.id.clone(),
                expected_versions: Vec::new(),
            },
        )
        .expect("apply the follow-up")
    };
    assert!(applied.created);
    assert_eq!(applied.entity_type, ProposalEntityType::Task);
    assert_eq!(open_task_count(&storage), 1);

    let task = {
        let guard = storage.lock().expect("storage lock");
        application::get_task(&guard, &applied.entity_id).expect("read the task")
    };
    assert_eq!(task.title, "chase the proposal I sent");
    assert_eq!(task.body.as_deref(), Some(draft.draft_text.as_str()));
    assert_eq!(task.parent_id.as_deref(), Some(contact_id.as_str()));

    // Undo drops the task instead of deleting it — the history stays.
    let undone = {
        let mut guard = storage.lock().expect("storage lock");
        undo_proposal(
            &mut guard,
            &store,
            UndoProposalRequest {
                actor: Actor::User,
                undo_token: applied.undo_token.clone(),
                expected_versions: vec![RecordVersion {
                    entity_type: "task".into(),
                    entity_id: applied.entity_id.clone(),
                    version: applied.version,
                }],
            },
        )
        .expect("undo the follow-up")
    };
    assert_eq!(undone.action, "dropped");
    assert_eq!(open_task_count(&storage), 0);
    let guard = storage.lock().expect("storage lock");
    assert_eq!(
        application::get_task(&guard, &applied.entity_id)
            .expect("the task row is still there")
            .status,
        TaskStatus::Dropped
    );
}

#[test]
fn with_the_assistant_off_the_template_comes_back_verbatim_and_nothing_is_sent() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let credentials = InMemoryCredentialStore::with_key("sk-test");
    let store = ProposalStore::new();
    let contact_id = seed_contact(&storage, "Dana Ruiz");

    let draft = propose_followup(
        &storage,
        &credentials,
        &store,
        "contact",
        &contact_id,
        None,
        None,
    )
    .expect("template-only draft");

    assert!(!draft.used_provider);
    assert_eq!(draft.model, None);
    assert_eq!(draft.endpoint_host, None);
    assert!(draft.included_record_refs.is_empty());
    // No objective means the first template, used exactly as written.
    let expected = default_templates().remove(0);
    assert_eq!(draft.template_id, expected.id);
    assert_eq!(draft.draft_text, expected.body);
    assert_eq!(draft.proposal.kind, ProposalKind::CreateFollowupTask);
    assert_eq!(draft.proposal.summary, "Follow up with Dana Ruiz");
    // The assistant being off means the credential store is never touched.
    assert_eq!(credentials.read_count(), 0);
    assert_eq!(open_task_count(&storage), 0);

    // The template-only draft applies through the same path.
    let mut guard = storage.lock().expect("storage lock");
    let applied = apply_proposal(
        &mut guard,
        &store,
        ApplyProposalRequest {
            actor: Actor::User,
            proposal_id: draft.proposal.id.clone(),
            expected_versions: Vec::new(),
        },
    )
    .expect("apply the follow-up");
    assert_eq!(
        application::get_task(&guard, &applied.entity_id)
            .expect("read the task")
            .body
            .as_deref(),
        Some(expected.body.as_str())
    );
}

#[test]
fn an_empty_model_answer_falls_back_to_the_template_with_a_warning() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let store = ProposalStore::new();
    let contact_id = seed_contact(&storage, "Dana Ruiz");

    let provider = CannedProvider::new("   \n  ");
    let draft = propose_followup_with(
        &storage,
        Some(&provider),
        &store,
        "contact",
        &contact_id,
        None,
        None,
    )
    .expect("draft still comes back");

    assert!(!draft.used_provider);
    assert_eq!(draft.draft_text, default_templates().remove(0).body);
    assert_eq!(draft.warnings.len(), 1);
    assert_eq!(draft.proposal.warnings, draft.warnings);
}

#[test]
fn drafting_a_follow_up_for_an_unknown_record_is_not_found() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let credentials = InMemoryCredentialStore::new();
    let store = ProposalStore::new();

    assert_eq!(
        propose_followup(
            &storage,
            &credentials,
            &store,
            "contact",
            "missing",
            None,
            None
        )
        .expect_err("no such contact")
        .kind(),
        "not_found"
    );
    assert_eq!(store.pending_count(), 0);
}
