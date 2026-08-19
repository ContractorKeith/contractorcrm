//! Typed proposal engine — plain-language drafts that never write.
//!
//! The split is deliberate and is the whole point of this module:
//!
//! * `propose_record` / `propose_update` read the database, ask the configured
//!   model for a structured draft, validate it with the SAME rules the direct
//!   create/update commands use, and hand back a typed diff. They never write.
//! * `apply_proposal` is the only writer. It re-checks expected versions and
//!   re-runs validation against current data, then applies the draft through
//!   the ordinary application-layer commands, so a proposal can never take a
//!   shortcut around a rule the manual path enforces.
//! * `undo_proposal` reverses one applied proposal: a created record is
//!   archived (never hard-deleted), an updated record goes back to its stored
//!   before-image.
//!
//! Drafts live in memory only — never in SQLite — and expire after 15 minutes,
//! so a stale draft can't be applied against data that moved on.
//!
//! Mutex rule (from the provider seam): the model call runs with no storage
//! lock held. Every flow here is lock → snapshot → unlock → call → lock.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::ai::{
    configured_provider, CompletionProvider, ContextPreview, CredentialStore, ProviderRequest,
    RecordRef,
};
use crate::application::{
    self, ArchiveRequest, ChannelInput, CompanyPatch, ContactPatch, CreateCompanyRequest,
    CreateContactRequest, CreateOpportunityRequest, CreateTaskRequest, OpportunityPatch,
    TaskActionRequest, TaskPatch, UpdateCompanyRequest, UpdateContactRequest,
    UpdateOpportunityRequest,
};
use crate::domain::{Actor, Company, Contact, Opportunity};
use crate::error::ApplicationError;
use crate::storage::{new_id, Storage};

/// How long a draft (or an undo token) stays usable.
pub const PROPOSAL_TTL_MINUTES: i64 = 15;

/// Longest plain-language ask accepted; keeps prompts (and context) bounded.
const MAX_DESCRIPTION_CHARS: usize = 2000;

/// Most warnings ever returned, so a hostile model answer cannot flood the UI.
const MAX_WARNINGS: usize = 12;

/// Longest single value shown in the bounded record projection sent to the
/// model. Notes can be long; the model does not need all of one.
const MAX_PROJECTION_VALUE_CHARS: usize = 200;

/// Output cap for the extraction call — a draft is a small JSON object.
const MAX_OUTPUT_TOKENS: u32 = 800;

/// Draft-side caps on model-supplied text. Free-form note fields get room to
/// be useful; every other field is a name, an address part, or a code. The
/// validators still have the final say — these only stop an unbounded string
/// from reaching them in the first place.
const MAX_DRAFT_NOTE_CHARS: usize = 10_000;
const MAX_DRAFT_TEXT_CHARS: usize = 500;

fn draft_field_cap(key: &str) -> usize {
    match key {
        "notes" | "licenseNotes" => MAX_DRAFT_NOTE_CHARS,
        _ => MAX_DRAFT_TEXT_CHARS,
    }
}

// ---------------------------------------------------------------------------
// Prompts (kept together so wording can be tuned in one place)
// ---------------------------------------------------------------------------

const EXTRACTION_SYSTEM_PROMPT: &str = "\
You turn a contractor's plain-language note into one JSON object for their CRM.
Reply with a single JSON object and nothing else: no prose, no explanation, no
code fences. Use only the field names listed in the request. Leave out any
field the note does not clearly state — never invent names, phone numbers,
email addresses, street addresses, dates, or dollar amounts.";

const CONTACT_FIELDS: &str = "\
firstName (text), lastName (text), displayName (text), companyName (text, the
company this person works for), role (one of owner, estimator, site_contact,
office, other), kind (one of client, lead, sub, vendor, supplier, other),
phone (text), email (text), preferredContactMethod (text), addressLine1 (text),
addressLine2 (text), city (text), state (text), postalCode (text),
propertyType (text), notes (text)";

const COMPANY_FIELDS: &str = "\
name (text), kind (one of client, lead, sub, vendor, supplier, other),
phone (text), email (text), website (text), addressLine1 (text),
addressLine2 (text), city (text), state (text), postalCode (text),
serviceArea (text), licenseNotes (text), notes (text)";

const OPPORTUNITY_FIELDS: &str = "\
name (text, what the job is), contactName (text, the person it is for),
companyName (text, the company it is for), valueMinor (whole number of cents,
so $4,500 is 450000), currencyCode (three letters, e.g. USD),
probabilityPercent (whole number 0-100), expectedCloseDate (YYYY-MM-DD),
source (one of referral, repeat_client, website, sign, other),
sourceLabel (text), notes (text)";

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

/// Which record surface a proposal is about. `Task` only ever arrives through
/// the follow-up drafting seam — plain-language create/update drafts refuse it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalEntityType {
    Contact,
    Company,
    Opportunity,
    Task,
}

impl ProposalEntityType {
    pub(crate) fn as_wire_value(self) -> &'static str {
        match self {
            Self::Contact => "contact",
            Self::Company => "company",
            Self::Opportunity => "opportunity",
            Self::Task => "task",
        }
    }
}

/// What a proposal would do. New kinds are added here without changing
/// anything already published.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalKind {
    CreateContact,
    CreateCompany,
    CreateOpportunity,
    UpdateContact,
    UpdateCompany,
    UpdateOpportunity,
    /// A follow-up task drafted from a template (see `followups.rs`).
    CreateFollowupTask,
}

/// One field the proposal would change. `before` is absent on a create.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldChange {
    pub field: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

/// A record and the version the caller believes it is at. Used both as the
/// proposal's affected-version list and as the caller's apply/undo guard.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordVersion {
    pub entity_type: String,
    pub entity_id: String,
    pub version: i64,
}

/// A validated draft waiting for an explicit apply. The id is opaque and only
/// meaningful to the in-memory store.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Proposal {
    pub id: String,
    pub kind: ProposalKind,
    pub entity_type: ProposalEntityType,
    /// The record being changed; absent for a create.
    pub entity_id: Option<String>,
    /// One contractor-facing line describing the draft.
    pub summary: String,
    pub changes: Vec<FieldChange>,
    pub warnings: Vec<String>,
    pub affected_versions: Vec<RecordVersion>,
    pub created_at: String,
    pub expires_at: String,
}

/// What an applied proposal did, plus the token that reverses it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposalApplied {
    pub entity_type: ProposalEntityType,
    pub entity_id: String,
    /// True when the record did not exist before.
    pub created: bool,
    pub version: i64,
    pub undo_token: String,
    pub undo_expires_at: String,
}

/// What an undo did. `action` is `archived` (undoing a create) or `reverted`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposalUndone {
    pub entity_type: ProposalEntityType,
    pub entity_id: String,
    pub action: String,
    pub version: i64,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplyProposalRequest {
    #[serde(default)]
    pub actor: Actor,
    pub proposal_id: String,
    #[serde(default)]
    pub expected_versions: Vec<RecordVersion>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UndoProposalRequest {
    #[serde(default)]
    pub actor: Actor,
    pub undo_token: String,
    #[serde(default)]
    pub expected_versions: Vec<RecordVersion>,
}

// ---------------------------------------------------------------------------
// In-memory store
// ---------------------------------------------------------------------------

/// The typed change a proposal would apply, kept out of the wire shape so the
/// payload can never be supplied by a caller.
#[derive(Clone, Debug)]
enum ProposalPayload {
    CreateContact(ContactPatch),
    CreateCompany(CompanyPatch),
    CreateOpportunity(OpportunityPatch),
    UpdateContact {
        contact_id: String,
        after: ContactPatch,
        before: ContactPatch,
    },
    UpdateCompany {
        company_id: String,
        after: CompanyPatch,
        before: CompanyPatch,
    },
    UpdateOpportunity {
        opportunity_id: String,
        after: OpportunityPatch,
        before: OpportunityPatch,
    },
    /// A follow-up task drafted from a template; applying creates the task.
    CreateFollowupTask(TaskPatch),
}

/// How an applied proposal is reversed.
#[derive(Clone, Debug)]
enum UndoAction {
    /// Undoing a create archives the new record — history is never destroyed.
    ArchiveRecord {
        entity_type: ProposalEntityType,
        entity_id: String,
    },
    RevertContact {
        contact_id: String,
        before: ContactPatch,
    },
    RevertCompany {
        company_id: String,
        before: CompanyPatch,
    },
    RevertOpportunity {
        opportunity_id: String,
        before: OpportunityPatch,
    },
    /// Undoing a created follow-up task drops it — tasks have no archive flag,
    /// and dropping keeps the row (and its history) instead of deleting it.
    DropTask { task_id: String },
}

impl UndoAction {
    fn entity_type(&self) -> ProposalEntityType {
        match self {
            Self::ArchiveRecord { entity_type, .. } => *entity_type,
            Self::RevertContact { .. } => ProposalEntityType::Contact,
            Self::RevertCompany { .. } => ProposalEntityType::Company,
            Self::RevertOpportunity { .. } => ProposalEntityType::Opportunity,
            Self::DropTask { .. } => ProposalEntityType::Task,
        }
    }

    fn entity_id(&self) -> &str {
        match self {
            Self::ArchiveRecord { entity_id, .. } => entity_id,
            Self::RevertContact { contact_id, .. } => contact_id,
            Self::RevertCompany { company_id, .. } => company_id,
            Self::RevertOpportunity { opportunity_id, .. } => opportunity_id,
            Self::DropTask { task_id } => task_id,
        }
    }
}

#[derive(Clone, Debug)]
struct StoredProposal {
    proposal: Proposal,
    payload: ProposalPayload,
    expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
struct StoredUndo {
    action: UndoAction,
    /// The record's version right after the apply. Undo refuses to run once
    /// the record has moved past it — reverting over newer work would be
    /// exactly the silent overwrite version checks exist to prevent.
    version_after_apply: i64,
    expires_at: DateTime<Utc>,
}

#[derive(Default)]
struct StoreInner {
    proposals: HashMap<String, StoredProposal>,
    undos: HashMap<String, StoredUndo>,
}

/// Drafts and undo tokens, in memory only. Managed as Tauri state next to the
/// storage mutex; nothing here survives a restart, which is the point.
pub struct ProposalStore {
    inner: Mutex<StoreInner>,
    ttl: Duration,
}

impl Default for ProposalStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ProposalStore {
    pub fn new() -> Self {
        Self::with_ttl(Duration::minutes(PROPOSAL_TTL_MINUTES))
    }

    /// Custom lifetime — tests use a zero TTL to exercise expiry.
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            inner: Mutex::new(StoreInner::default()),
            ttl,
        }
    }

    /// Number of drafts still held; drops expired entries first.
    pub fn pending_count(&self) -> usize {
        let mut inner = self.lock();
        purge(&mut inner, Utc::now());
        inner.proposals.len()
    }

    fn lock(&self) -> MutexGuard<'_, StoreInner> {
        self.inner.lock().expect("proposal store mutex poisoned")
    }

    fn expiry(&self) -> DateTime<Utc> {
        Utc::now() + self.ttl
    }

    fn insert(&self, stored: StoredProposal) {
        let mut inner = self.lock();
        purge(&mut inner, Utc::now());
        inner.proposals.insert(stored.proposal.id.clone(), stored);
    }

    /// Single use: taking a draft removes it. Unknown or expired ids look the
    /// same to the caller on purpose.
    fn take(&self, proposal_id: &str) -> Result<StoredProposal, ApplicationError> {
        let mut inner = self.lock();
        purge(&mut inner, Utc::now());
        inner
            .proposals
            .remove(proposal_id)
            .ok_or_else(|| ApplicationError::ProposalExpired {
                proposal_id: proposal_id.to_owned(),
            })
    }

    /// Return a draft that could not be applied (a version conflict is the
    /// caller's problem to fix, not a reason to lose their draft).
    fn put_back(&self, stored: StoredProposal) {
        let mut inner = self.lock();
        purge(&mut inner, Utc::now());
        inner.proposals.insert(stored.proposal.id.clone(), stored);
    }

    fn insert_undo(&self, token: String, stored: StoredUndo) {
        let mut inner = self.lock();
        purge(&mut inner, Utc::now());
        inner.undos.insert(token, stored);
    }

    fn take_undo(&self, token: &str) -> Result<StoredUndo, ApplicationError> {
        let mut inner = self.lock();
        purge(&mut inner, Utc::now());
        inner
            .undos
            .remove(token)
            .ok_or_else(|| ApplicationError::ProposalExpired {
                proposal_id: token.to_owned(),
            })
    }

    fn put_back_undo(&self, token: String, stored: StoredUndo) {
        let mut inner = self.lock();
        purge(&mut inner, Utc::now());
        inner.undos.insert(token, stored);
    }
}

fn purge(inner: &mut StoreInner, now: DateTime<Utc>) {
    inner.proposals.retain(|_, stored| stored.expires_at > now);
    inner.undos.retain(|_, stored| stored.expires_at > now);
}

// ---------------------------------------------------------------------------
// Propose (read-only)
// ---------------------------------------------------------------------------

fn lock_storage(storage: &Mutex<Storage>) -> MutexGuard<'_, Storage> {
    storage.lock().expect("storage mutex poisoned")
}

/// Load the configured provider or explain that the assistant is off. Called
/// with the storage lock held; the returned provider owns everything it needs.
fn require_provider(
    storage: &Storage,
    credentials: &dyn CredentialStore,
) -> Result<Box<dyn CompletionProvider>, ApplicationError> {
    match configured_provider(storage, credentials)? {
        Some(provider) => Ok(Box::new(provider)),
        None => Err(ApplicationError::ProviderUnavailable {
            reason: "Turn the AI assistant on in Settings before asking it to draft a record."
                .into(),
        }),
    }
}

/// Draft a new contact, company, or opportunity from a plain-language note.
/// Writes nothing.
pub fn propose_record(
    storage: &Mutex<Storage>,
    credentials: &dyn CredentialStore,
    store: &ProposalStore,
    kind: ProposalEntityType,
    description: &str,
) -> Result<Proposal, ApplicationError> {
    let provider = {
        let guard = lock_storage(storage);
        require_provider(&guard, credentials)?
    };
    propose_record_with_provider(storage, provider.as_ref(), store, kind, description)
}

/// Same flow against an explicit provider — the seam tests use.
pub fn propose_record_with_provider(
    storage: &Mutex<Storage>,
    provider: &dyn CompletionProvider,
    store: &ProposalStore,
    kind: ProposalEntityType,
    description: &str,
) -> Result<Proposal, ApplicationError> {
    let description = checked_description(description)?;

    // No lock is held across the model call.
    let completion = provider.complete(&ProviderRequest {
        purpose: "propose_record".into(),
        system_text: EXTRACTION_SYSTEM_PROMPT.into(),
        user_text: create_prompt(kind, &description)?,
        context_text: None,
        included_record_refs: Vec::new(),
        max_output_tokens: Some(MAX_OUTPUT_TOKENS),
        timeout_seconds: None,
    })?;

    let mut draft = Draft::parse(&completion.text)?;
    let guard = lock_storage(storage);
    let (payload, proposal_kind, label) = match kind {
        ProposalEntityType::Contact => {
            let mut patch = ContactPatch::default();
            apply_contact_draft(&guard, &mut draft, &mut patch)?;
            application::check_contact_patch(&patch)?;
            let label = contact_label(&patch);
            (
                ProposalPayload::CreateContact(patch),
                ProposalKind::CreateContact,
                label,
            )
        }
        ProposalEntityType::Company => {
            let mut patch = CompanyPatch::default();
            apply_company_draft(&mut draft, &mut patch);
            application::check_company_patch(&patch)?;
            let label = patch.name.clone();
            (
                ProposalPayload::CreateCompany(patch),
                ProposalKind::CreateCompany,
                label,
            )
        }
        ProposalEntityType::Opportunity => {
            let mut patch = OpportunityPatch::default();
            apply_opportunity_draft(&guard, &mut draft, &mut patch)?;
            application::check_opportunity_patch(&patch)?;
            let label = patch.name.clone();
            (
                ProposalPayload::CreateOpportunity(patch),
                ProposalKind::CreateOpportunity,
                label,
            )
        }
        // Unreachable: `draft_fields` above already rejected a task draft.
        ProposalEntityType::Task => return Err(unsupported_task_draft()),
    };
    drop(guard);

    let changes = match &payload {
        ProposalPayload::CreateContact(patch) => diff(None, &contact_projection(patch)),
        ProposalPayload::CreateCompany(patch) => diff(None, &company_projection(patch)),
        ProposalPayload::CreateOpportunity(patch) => diff(None, &opportunity_projection(patch)),
        _ => Vec::new(),
    };

    Ok(finish_proposal(
        store,
        payload,
        proposal_kind,
        kind,
        None,
        format!("Create {} \"{label}\"", kind.as_wire_value()),
        changes,
        draft.into_warnings(),
        Vec::new(),
    ))
}

/// Draft a change to an existing record from a plain-language request. Writes
/// nothing; the diff is computed here, deterministically, against the record
/// as it is right now.
pub fn propose_update(
    storage: &Mutex<Storage>,
    credentials: &dyn CredentialStore,
    store: &ProposalStore,
    target: (ProposalEntityType, &str, i64),
    request: &str,
) -> Result<Proposal, ApplicationError> {
    let provider = {
        let guard = lock_storage(storage);
        require_provider(&guard, credentials)?
    };
    propose_update_with_provider(storage, provider.as_ref(), store, target, request)
}

/// Same flow against an explicit provider — the seam tests use.
pub fn propose_update_with_provider(
    storage: &Mutex<Storage>,
    provider: &dyn CompletionProvider,
    store: &ProposalStore,
    target: (ProposalEntityType, &str, i64),
    request: &str,
) -> Result<Proposal, ApplicationError> {
    let (entity_type, entity_id, expected_version) = target;
    let request = checked_description(request)?;

    // Snapshot the record under the lock, then let go of it for the call.
    let snapshot = {
        let guard = lock_storage(storage);
        load_snapshot(&guard, entity_type, entity_id, expected_version)?
    };

    let completion = provider.complete(&ProviderRequest {
        purpose: "propose_update".into(),
        system_text: EXTRACTION_SYSTEM_PROMPT.into(),
        user_text: update_prompt(entity_type, &request)?,
        context_text: Some(snapshot.context.clone()),
        included_record_refs: vec![RecordRef {
            entity_type: entity_type.as_wire_value().into(),
            entity_id: entity_id.to_owned(),
            label: snapshot.label.clone(),
        }],
        max_output_tokens: Some(MAX_OUTPUT_TOKENS),
        timeout_seconds: None,
    })?;

    let mut draft = Draft::parse(&completion.text)?;
    let guard = lock_storage(storage);
    let (payload, proposal_kind, changes) = match snapshot.before.clone() {
        RecordPatch::Contact(before) => {
            let mut after = before.clone();
            // Links stay put: a plain-language edit never re-parents a record.
            apply_contact_draft_fields(&mut draft, &mut after);
            application::check_contact_patch(&after)?;
            let changes = diff(
                Some(&contact_projection(&before)),
                &contact_projection(&after),
            );
            (
                ProposalPayload::UpdateContact {
                    contact_id: entity_id.to_owned(),
                    after,
                    before,
                },
                ProposalKind::UpdateContact,
                changes,
            )
        }
        RecordPatch::Company(before) => {
            let mut after = before.clone();
            apply_company_draft(&mut draft, &mut after);
            application::check_company_patch(&after)?;
            let changes = diff(
                Some(&company_projection(&before)),
                &company_projection(&after),
            );
            (
                ProposalPayload::UpdateCompany {
                    company_id: entity_id.to_owned(),
                    after,
                    before,
                },
                ProposalKind::UpdateCompany,
                changes,
            )
        }
        RecordPatch::Opportunity(before) => {
            let mut after = before.clone();
            apply_opportunity_draft_fields(&mut draft, &mut after);
            application::check_opportunity_patch(&after)?;
            let changes = diff(
                Some(&opportunity_projection(&before)),
                &opportunity_projection(&after),
            );
            (
                ProposalPayload::UpdateOpportunity {
                    opportunity_id: entity_id.to_owned(),
                    after,
                    before,
                },
                ProposalKind::UpdateOpportunity,
                changes,
            )
        }
    };
    drop(guard);

    let mut warnings = draft.into_warnings();
    if changes.is_empty() {
        warnings.push("Nothing in this record needed to change.".into());
    }

    Ok(finish_proposal(
        store,
        payload,
        proposal_kind,
        entity_type,
        Some(entity_id.to_owned()),
        format!(
            "Update {} \"{}\"",
            entity_type.as_wire_value(),
            snapshot.label
        ),
        changes,
        warnings,
        vec![RecordVersion {
            entity_type: entity_type.as_wire_value().into(),
            entity_id: entity_id.to_owned(),
            version: snapshot.version,
        }],
    ))
}

/// The same bounded record projection `propose_update` would send, built
/// without a provider and without reading credentials. The caller's version is
/// still checked, so a preview cannot quietly describe a stale record.
pub fn preview_update_context(
    storage: &Storage,
    entity_type: ProposalEntityType,
    entity_id: &str,
    expected_version: i64,
) -> Result<ContextPreview, ApplicationError> {
    let snapshot = load_snapshot(storage, entity_type, entity_id, expected_version)?;
    Ok(ContextPreview {
        purpose: "propose_update".into(),
        context_text: snapshot.context,
        included_record_refs: vec![RecordRef {
            entity_type: entity_type.as_wire_value().into(),
            entity_id: entity_id.to_owned(),
            label: snapshot.label,
        }],
    })
}

/// Assemble, store, and return the draft. Kept in one place so every proposal
/// gets the same id, timestamps, and TTL treatment.
#[allow(clippy::too_many_arguments)]
fn finish_proposal(
    store: &ProposalStore,
    payload: ProposalPayload,
    kind: ProposalKind,
    entity_type: ProposalEntityType,
    entity_id: Option<String>,
    summary: String,
    changes: Vec<FieldChange>,
    warnings: Vec<String>,
    affected_versions: Vec<RecordVersion>,
) -> Proposal {
    let expires_at = store.expiry();
    let proposal = Proposal {
        id: new_id(),
        kind,
        entity_type,
        entity_id,
        summary,
        changes,
        warnings,
        affected_versions,
        created_at: iso(Utc::now()),
        expires_at: iso(expires_at),
    };
    store.insert(StoredProposal {
        proposal: proposal.clone(),
        payload,
        expires_at,
    });
    proposal
}

// ---------------------------------------------------------------------------
// Apply and undo (the only writers)
// ---------------------------------------------------------------------------

/// Apply a draft. Re-checks versions, re-runs validation through the ordinary
/// create/update commands, and returns an undo token.
pub fn apply_proposal(
    storage: &mut Storage,
    store: &ProposalStore,
    request: ApplyProposalRequest,
) -> Result<ProposalApplied, ApplicationError> {
    let stored = store.take(&request.proposal_id)?;

    // A conflict must not cost the user their draft: check first, put back on
    // failure, and only then write.
    if let Err(error) = check_expected_versions(storage, &stored, &request.expected_versions) {
        store.put_back(stored);
        return Err(error);
    }

    let actor = request.actor;
    let outcome = apply_payload(storage, actor, &stored.payload);
    let (applied, undo) = match outcome {
        Ok(result) => result,
        Err(error) => {
            // Validation ran again against current data and rejected the
            // draft — hand it back so the user can see why and re-ask.
            store.put_back(stored);
            return Err(error);
        }
    };

    // The undo token is registered BEFORE the audit row: the record write has
    // already committed, so losing the only way to reverse it because a log
    // row failed would be the worse of the two failures.
    let undo_expires_at = store.expiry();
    store.insert_undo(
        applied.undo_token.clone(),
        StoredUndo {
            action: undo,
            version_after_apply: applied.version,
            expires_at: undo_expires_at,
        },
    );

    // The record write already committed in its own transaction with its own
    // command_log row; this extra row records that a draft (not a hand edit)
    // was applied. Best-effort after the fact, like `mcp::log_agent_call`.
    log_event_best_effort(
        storage,
        actor,
        applied.entity_type,
        &applied.entity_id,
        &format!("applied the assistant's draft: {}", stored.proposal.summary),
    );
    Ok(ProposalApplied {
        undo_expires_at: iso(undo_expires_at),
        ..applied
    })
}

/// Reverse one applied proposal: archive what it created, or put back what it
/// changed. Single use, version-checked, and audited like any other write.
pub fn undo_proposal(
    storage: &mut Storage,
    store: &ProposalStore,
    request: UndoProposalRequest,
) -> Result<ProposalUndone, ApplicationError> {
    let stored = store.take_undo(&request.undo_token)?;
    let entity_type = stored.action.entity_type();
    let entity_id = stored.action.entity_id().to_owned();

    let current = match current_version(storage, entity_type, &entity_id) {
        Ok(version) => version,
        Err(error) => {
            store.put_back_undo(request.undo_token.clone(), stored);
            return Err(error);
        }
    };
    // The record must still be exactly where the apply left it. This check is
    // unconditional: a caller that asserts nothing (a future agent client) must
    // not get a silent revert over work done after the apply.
    if current != stored.version_after_apply {
        let expected = stored.version_after_apply;
        store.put_back_undo(request.undo_token.clone(), stored);
        return Err(version_conflict(entity_type, &entity_id, expected, current));
    }
    // Anything the caller asserted is an extra guard on top, never a substitute.
    if let Some(expected) = expected_for(&request.expected_versions, entity_type, &entity_id) {
        if expected != current {
            store.put_back_undo(request.undo_token.clone(), stored);
            return Err(version_conflict(entity_type, &entity_id, expected, current));
        }
    }

    let actor = request.actor;
    let undone = match undo_action(storage, actor, &stored.action, current) {
        Ok(undone) => undone,
        Err(error) => {
            store.put_back_undo(request.undo_token.clone(), stored);
            return Err(error);
        }
    };
    log_event_best_effort(
        storage,
        actor,
        entity_type,
        &entity_id,
        "undid the assistant's applied draft",
    );
    Ok(undone)
}

fn apply_payload(
    storage: &mut Storage,
    actor: Actor,
    payload: &ProposalPayload,
) -> Result<(ProposalApplied, UndoAction), ApplicationError> {
    let token = new_id();
    match payload.clone() {
        ProposalPayload::CreateContact(contact) => {
            let created =
                application::create_contact(storage, CreateContactRequest { actor, contact })?;
            Ok((
                applied(
                    ProposalEntityType::Contact,
                    &created.id,
                    true,
                    created.version,
                    &token,
                ),
                UndoAction::ArchiveRecord {
                    entity_type: ProposalEntityType::Contact,
                    entity_id: created.id,
                },
            ))
        }
        ProposalPayload::CreateCompany(company) => {
            let created =
                application::create_company(storage, CreateCompanyRequest { actor, company })?;
            Ok((
                applied(
                    ProposalEntityType::Company,
                    &created.id,
                    true,
                    created.version,
                    &token,
                ),
                UndoAction::ArchiveRecord {
                    entity_type: ProposalEntityType::Company,
                    entity_id: created.id,
                },
            ))
        }
        ProposalPayload::CreateOpportunity(opportunity) => {
            let created = application::create_opportunity(
                storage,
                CreateOpportunityRequest {
                    actor,
                    stage_id: None,
                    opportunity,
                },
            )?;
            Ok((
                applied(
                    ProposalEntityType::Opportunity,
                    &created.id,
                    true,
                    created.version,
                    &token,
                ),
                UndoAction::ArchiveRecord {
                    entity_type: ProposalEntityType::Opportunity,
                    entity_id: created.id,
                },
            ))
        }
        ProposalPayload::UpdateContact {
            contact_id,
            after,
            before,
        } => {
            let expected_version =
                current_version(storage, ProposalEntityType::Contact, &contact_id)?;
            let updated = application::update_contact(
                storage,
                UpdateContactRequest {
                    actor,
                    contact_id: contact_id.clone(),
                    expected_version,
                    patch: after,
                },
            )?;
            Ok((
                applied(
                    ProposalEntityType::Contact,
                    &updated.id,
                    false,
                    updated.version,
                    &token,
                ),
                UndoAction::RevertContact { contact_id, before },
            ))
        }
        ProposalPayload::UpdateCompany {
            company_id,
            after,
            before,
        } => {
            let expected_version =
                current_version(storage, ProposalEntityType::Company, &company_id)?;
            let updated = application::update_company(
                storage,
                UpdateCompanyRequest {
                    actor,
                    company_id: company_id.clone(),
                    expected_version,
                    patch: after,
                },
            )?;
            Ok((
                applied(
                    ProposalEntityType::Company,
                    &updated.id,
                    false,
                    updated.version,
                    &token,
                ),
                UndoAction::RevertCompany { company_id, before },
            ))
        }
        ProposalPayload::UpdateOpportunity {
            opportunity_id,
            after,
            before,
        } => {
            let expected_version =
                current_version(storage, ProposalEntityType::Opportunity, &opportunity_id)?;
            let updated = application::update_opportunity(
                storage,
                UpdateOpportunityRequest {
                    actor,
                    opportunity_id: opportunity_id.clone(),
                    expected_version,
                    patch: after,
                },
            )?;
            Ok((
                applied(
                    ProposalEntityType::Opportunity,
                    &updated.id,
                    false,
                    updated.version,
                    &token,
                ),
                UndoAction::RevertOpportunity {
                    opportunity_id,
                    before,
                },
            ))
        }
        ProposalPayload::CreateFollowupTask(task) => {
            let created = application::create_task(storage, CreateTaskRequest { actor, task })?;
            Ok((
                applied(
                    ProposalEntityType::Task,
                    &created.id,
                    true,
                    created.version,
                    &token,
                ),
                UndoAction::DropTask {
                    task_id: created.id,
                },
            ))
        }
    }
}

fn undo_action(
    storage: &mut Storage,
    actor: Actor,
    action: &UndoAction,
    expected_version: i64,
) -> Result<ProposalUndone, ApplicationError> {
    match action.clone() {
        UndoAction::ArchiveRecord {
            entity_type,
            entity_id,
        } => {
            let request = ArchiveRequest {
                actor,
                id: entity_id.clone(),
                expected_version,
            };
            let version = match entity_type {
                ProposalEntityType::Contact => {
                    application::archive_contact(storage, request)?.version
                }
                ProposalEntityType::Company => {
                    application::archive_company(storage, request)?.version
                }
                ProposalEntityType::Opportunity => {
                    application::archive_opportunity(storage, request)?.version
                }
                // Tasks are undone by dropping them, never by this arm.
                ProposalEntityType::Task => return Err(unsupported_task_draft()),
            };
            Ok(ProposalUndone {
                entity_type,
                entity_id,
                action: "archived".into(),
                version,
            })
        }
        UndoAction::RevertContact { contact_id, before } => {
            let reverted = application::update_contact(
                storage,
                UpdateContactRequest {
                    actor,
                    contact_id: contact_id.clone(),
                    expected_version,
                    patch: before,
                },
            )?;
            Ok(reverted_result(
                ProposalEntityType::Contact,
                contact_id,
                reverted.version,
            ))
        }
        UndoAction::RevertCompany { company_id, before } => {
            let reverted = application::update_company(
                storage,
                UpdateCompanyRequest {
                    actor,
                    company_id: company_id.clone(),
                    expected_version,
                    patch: before,
                },
            )?;
            Ok(reverted_result(
                ProposalEntityType::Company,
                company_id,
                reverted.version,
            ))
        }
        UndoAction::RevertOpportunity {
            opportunity_id,
            before,
        } => {
            let reverted = application::update_opportunity(
                storage,
                UpdateOpportunityRequest {
                    actor,
                    opportunity_id: opportunity_id.clone(),
                    expected_version,
                    patch: before,
                },
            )?;
            Ok(reverted_result(
                ProposalEntityType::Opportunity,
                opportunity_id,
                reverted.version,
            ))
        }
        UndoAction::DropTask { task_id } => {
            let dropped = application::drop_task(
                storage,
                TaskActionRequest {
                    actor,
                    task_id: task_id.clone(),
                    expected_version,
                },
            )?;
            Ok(ProposalUndone {
                entity_type: ProposalEntityType::Task,
                entity_id: task_id,
                action: "dropped".into(),
                version: dropped.version,
            })
        }
    }
}

fn applied(
    entity_type: ProposalEntityType,
    entity_id: &str,
    created: bool,
    version: i64,
    undo_token: &str,
) -> ProposalApplied {
    ProposalApplied {
        entity_type,
        entity_id: entity_id.to_owned(),
        created,
        version,
        undo_token: undo_token.to_owned(),
        undo_expires_at: String::new(),
    }
}

fn reverted_result(
    entity_type: ProposalEntityType,
    entity_id: String,
    version: i64,
) -> ProposalUndone {
    ProposalUndone {
        entity_type,
        entity_id,
        action: "reverted".into(),
        version,
    }
}

/// Compare every version the caller asserted (and, for an update, the version
/// the draft was built against) with the database as it is now.
fn check_expected_versions(
    storage: &Storage,
    stored: &StoredProposal,
    expected_versions: &[RecordVersion],
) -> Result<(), ApplicationError> {
    let mut asserted = expected_versions.to_vec();
    for affected in &stored.proposal.affected_versions {
        // Match on the whole identity: ids are unique per type, not globally,
        // so a contact assertion must never satisfy a company's affected row.
        if !asserted.iter().any(|entry| {
            entry.entity_id == affected.entity_id && entry.entity_type == affected.entity_type
        }) {
            asserted.push(affected.clone());
        }
    }
    for entry in asserted {
        let entity_type = parse_entity_type(&entry.entity_type)?;
        let current = current_version(storage, entity_type, &entry.entity_id)?;
        if current != entry.version {
            return Err(version_conflict(
                entity_type,
                &entry.entity_id,
                entry.version,
                current,
            ));
        }
    }
    Ok(())
}

fn expected_for(
    expected_versions: &[RecordVersion],
    entity_type: ProposalEntityType,
    entity_id: &str,
) -> Option<i64> {
    expected_versions
        .iter()
        .find(|entry| {
            entry.entity_id == entity_id && entry.entity_type == entity_type.as_wire_value()
        })
        .map(|entry| entry.version)
}

fn version_conflict(
    entity_type: ProposalEntityType,
    entity_id: &str,
    expected: i64,
    current: i64,
) -> ApplicationError {
    ApplicationError::VersionConflict {
        resource: resource_name(entity_type),
        id: entity_id.to_owned(),
        expected,
        current,
    }
}

fn resource_name(entity_type: ProposalEntityType) -> &'static str {
    match entity_type {
        ProposalEntityType::Contact => "contact",
        ProposalEntityType::Company => "company",
        ProposalEntityType::Opportunity => "opportunity",
        ProposalEntityType::Task => "task",
    }
}

fn parse_entity_type(value: &str) -> Result<ProposalEntityType, ApplicationError> {
    match value {
        "contact" => Ok(ProposalEntityType::Contact),
        "company" => Ok(ProposalEntityType::Company),
        "opportunity" => Ok(ProposalEntityType::Opportunity),
        "task" => Ok(ProposalEntityType::Task),
        _ => Err(ApplicationError::InvalidInput {
            field: "expectedVersions".into(),
            message: format!("unknown record type \"{value}\""),
        }),
    }
}

fn current_version(
    storage: &Storage,
    entity_type: ProposalEntityType,
    entity_id: &str,
) -> Result<i64, ApplicationError> {
    match entity_type {
        ProposalEntityType::Contact => Ok(application::get_contact(storage, entity_id)?.version),
        ProposalEntityType::Company => Ok(application::get_company(storage, entity_id)?.version),
        ProposalEntityType::Opportunity => Ok(application::get_opportunity(storage, entity_id)?
            .opportunity
            .version),
        ProposalEntityType::Task => Ok(application::get_task(storage, entity_id)?.version),
    }
}

/// One audit row saying a draft was applied or undone. Non-secret summary only.
///
/// Best-effort by design: the record write it describes already committed in
/// its own transaction (with its own `command_log` row), so a failure here is
/// reported on stderr rather than turned into an error that would strand an
/// applied change with no undo token. Same approach as `mcp::log_agent_call`.
fn log_event_best_effort(
    storage: &mut Storage,
    actor: Actor,
    entity_type: ProposalEntityType,
    entity_id: &str,
    summary: &str,
) {
    if let Err(error) = log_event(storage, actor, entity_type, entity_id, summary) {
        eprintln!(
            "ContractorCRM: could not record the proposal audit row for \
             {} {entity_id}: {error}",
            resource_name(entity_type)
        );
    }
}

fn log_event(
    storage: &mut Storage,
    actor: Actor,
    entity_type: ProposalEntityType,
    entity_id: &str,
    summary: &str,
) -> Result<(), ApplicationError> {
    let transaction = application::immediate(storage)?;
    application::log_command(
        &transaction,
        actor,
        resource_name(entity_type),
        entity_id,
        summary,
    )?;
    transaction.commit()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Record snapshots and projections
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum RecordPatch {
    Contact(ContactPatch),
    Company(CompanyPatch),
    Opportunity(OpportunityPatch),
}

struct Snapshot {
    before: RecordPatch,
    label: String,
    version: i64,
    context: String,
}

/// Read the record, check the caller's version, and build the bounded
/// projection the model is allowed to see (no attachments, no credentials).
fn load_snapshot(
    storage: &Storage,
    entity_type: ProposalEntityType,
    entity_id: &str,
    expected_version: i64,
) -> Result<Snapshot, ApplicationError> {
    let snapshot = match entity_type {
        ProposalEntityType::Contact => {
            let contact = application::get_contact(storage, entity_id)?;
            let patch = contact_patch_from(&contact);
            Snapshot {
                context: projection_text(&contact_projection(&patch)),
                label: contact.display_name.clone(),
                version: contact.version,
                before: RecordPatch::Contact(patch),
            }
        }
        ProposalEntityType::Company => {
            let company = application::get_company(storage, entity_id)?;
            let patch = company_patch_from(&company);
            Snapshot {
                context: projection_text(&company_projection(&patch)),
                label: company.name.clone(),
                version: company.version,
                before: RecordPatch::Company(patch),
            }
        }
        ProposalEntityType::Opportunity => {
            let detail = application::get_opportunity(storage, entity_id)?;
            let patch = opportunity_patch_from(&detail.opportunity);
            Snapshot {
                context: projection_text(&opportunity_projection(&patch)),
                label: detail.opportunity.name.clone(),
                version: detail.opportunity.version,
                before: RecordPatch::Opportunity(patch),
            }
        }
        ProposalEntityType::Task => return Err(unsupported_task_draft()),
    };
    if snapshot.version != expected_version {
        return Err(version_conflict(
            entity_type,
            entity_id,
            expected_version,
            snapshot.version,
        ));
    }
    Ok(snapshot)
}

fn contact_patch_from(contact: &Contact) -> ContactPatch {
    ContactPatch {
        company_id: contact.company_id.clone(),
        first_name: contact.first_name.clone(),
        last_name: contact.last_name.clone(),
        display_name: Some(contact.display_name.clone()),
        role: contact.role.map(|role| role.as_database_value().to_owned()),
        kind: contact.kind.as_database_value().to_owned(),
        preferred_contact_method: contact.preferred_contact_method.clone(),
        address_line1: contact.address_line1.clone(),
        address_line2: contact.address_line2.clone(),
        city: contact.city.clone(),
        state: contact.state.clone(),
        postal_code: contact.postal_code.clone(),
        property_type: contact.property_type.clone(),
        notes: contact.notes.clone(),
        favorite: contact.favorite,
        channels: contact
            .channels
            .iter()
            .map(|channel| ChannelInput {
                kind: channel.kind.as_database_value().to_owned(),
                label: channel.label.clone(),
                value: channel.value.clone(),
                preferred: channel.preferred,
            })
            .collect(),
    }
}

fn company_patch_from(company: &Company) -> CompanyPatch {
    CompanyPatch {
        name: company.name.clone(),
        kind: company.kind.as_database_value().to_owned(),
        phone: company.phone.clone(),
        email: company.email.clone(),
        website: company.website.clone(),
        address_line1: company.address_line1.clone(),
        address_line2: company.address_line2.clone(),
        city: company.city.clone(),
        state: company.state.clone(),
        postal_code: company.postal_code.clone(),
        service_area: company.service_area.clone(),
        license_notes: company.license_notes.clone(),
        notes: company.notes.clone(),
    }
}

fn opportunity_patch_from(opportunity: &Opportunity) -> OpportunityPatch {
    OpportunityPatch {
        name: opportunity.name.clone(),
        contact_id: opportunity.contact_id.clone(),
        company_id: opportunity.company_id.clone(),
        value_minor: opportunity.value.value_minor,
        currency_code: opportunity.value.currency_code.clone(),
        probability_percent: opportunity.probability_percent,
        expected_close_date: opportunity.expected_close_date.clone(),
        source: opportunity
            .source
            .map(|source| source.as_database_value().to_owned()),
        source_label: opportunity.source_label.clone(),
        notes: opportunity.notes.clone(),
    }
}

/// Ordered field projection — the single source for both the diff and the
/// context text, so the user sees exactly the fields the model saw.
type Projection = Vec<(&'static str, Option<String>)>;

fn contact_projection(patch: &ContactPatch) -> Projection {
    vec![
        ("firstName", text(&patch.first_name)),
        ("lastName", text(&patch.last_name)),
        ("displayName", text(&patch.display_name)),
        ("role", text(&patch.role)),
        ("kind", non_empty(&patch.kind)),
        ("phone", channel_value(patch, "phone")),
        ("email", channel_value(patch, "email")),
        (
            "preferredContactMethod",
            text(&patch.preferred_contact_method),
        ),
        ("addressLine1", text(&patch.address_line1)),
        ("addressLine2", text(&patch.address_line2)),
        ("city", text(&patch.city)),
        ("state", text(&patch.state)),
        ("postalCode", text(&patch.postal_code)),
        ("propertyType", text(&patch.property_type)),
        ("notes", text(&patch.notes)),
    ]
}

fn company_projection(patch: &CompanyPatch) -> Projection {
    vec![
        ("name", non_empty(&patch.name)),
        ("kind", non_empty(&patch.kind)),
        ("phone", text(&patch.phone)),
        ("email", text(&patch.email)),
        ("website", text(&patch.website)),
        ("addressLine1", text(&patch.address_line1)),
        ("addressLine2", text(&patch.address_line2)),
        ("city", text(&patch.city)),
        ("state", text(&patch.state)),
        ("postalCode", text(&patch.postal_code)),
        ("serviceArea", text(&patch.service_area)),
        ("licenseNotes", text(&patch.license_notes)),
        ("notes", text(&patch.notes)),
    ]
}

fn opportunity_projection(patch: &OpportunityPatch) -> Projection {
    vec![
        ("name", non_empty(&patch.name)),
        ("valueMinor", Some(patch.value_minor.to_string())),
        ("currencyCode", non_empty(&patch.currency_code)),
        (
            "probabilityPercent",
            patch.probability_percent.map(|value| value.to_string()),
        ),
        ("expectedCloseDate", text(&patch.expected_close_date)),
        ("source", text(&patch.source)),
        ("sourceLabel", text(&patch.source_label)),
        ("notes", text(&patch.notes)),
        ("contactId", text(&patch.contact_id)),
        ("companyId", text(&patch.company_id)),
    ]
}

fn task_projection(patch: &TaskPatch) -> Projection {
    vec![
        ("title", non_empty(&patch.title)),
        ("body", text(&patch.body)),
        ("dueAt", text(&patch.due_at)),
        ("remindAt", text(&patch.remind_at)),
        ("priority", text(&patch.priority)),
        ("parentType", text(&patch.parent_type)),
        ("parentId", text(&patch.parent_id)),
    ]
}

/// Build a follow-up task draft. Validated with exactly the rules
/// `create_task` runs, stored in the same TTL'd draft store, and applied and
/// undone through the same path as every other proposal. Writes nothing.
pub fn followup_task_proposal(
    store: &ProposalStore,
    task: TaskPatch,
    summary: String,
    warnings: Vec<String>,
) -> Result<Proposal, ApplicationError> {
    application::check_task_patch(&task)?;
    let changes = diff(None, &task_projection(&task));
    Ok(finish_proposal(
        store,
        ProposalPayload::CreateFollowupTask(task),
        ProposalKind::CreateFollowupTask,
        ProposalEntityType::Task,
        None,
        summary,
        changes,
        warnings,
        Vec::new(),
    ))
}

fn text(value: &Option<String>) -> Option<String> {
    value
        .as_ref()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn channel_value(patch: &ContactPatch, kind: &str) -> Option<String> {
    patch
        .channels
        .iter()
        .find(|channel| channel.kind == kind)
        .map(|channel| channel.value.clone())
}

/// Replace (or add) the first channel of a kind; contacts carry one preferred
/// phone and one preferred email in practice.
fn set_channel(patch: &mut ContactPatch, kind: &str, value: String) {
    if let Some(channel) = patch
        .channels
        .iter_mut()
        .find(|channel| channel.kind == kind)
    {
        channel.value = value;
        return;
    }
    patch.channels.push(ChannelInput {
        kind: kind.to_owned(),
        label: None,
        value,
        preferred: true,
    });
}

/// Field-by-field diff; only fields that actually change are reported.
fn diff(before: Option<&Projection>, after: &Projection) -> Vec<FieldChange> {
    after
        .iter()
        .filter_map(|(field, after_value)| {
            let before_value = before.and_then(|projection| {
                projection
                    .iter()
                    .find(|(name, _)| name == field)
                    .and_then(|(_, value)| value.clone())
            });
            if &before_value == after_value {
                return None;
            }
            Some(FieldChange {
                field: (*field).to_owned(),
                before: before_value,
                after: after_value.clone(),
            })
        })
        .collect()
}

fn projection_text(projection: &Projection) -> String {
    projection
        .iter()
        .filter_map(|(field, value)| {
            value.as_ref().map(|value| {
                let mut value = value.clone();
                if value.chars().count() > MAX_PROJECTION_VALUE_CHARS {
                    value = value
                        .chars()
                        .take(MAX_PROJECTION_VALUE_CHARS)
                        .collect::<String>()
                        + "…";
                }
                format!("{field}: {value}")
            })
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn contact_label(patch: &ContactPatch) -> String {
    text(&patch.display_name)
        .or_else(|| {
            let parts = [patch.first_name.as_deref(), patch.last_name.as_deref()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" ");
            non_empty(&parts)
        })
        .unwrap_or_else(|| "contact".to_owned())
}

// ---------------------------------------------------------------------------
// Extraction: model text in, typed patch out
// ---------------------------------------------------------------------------

fn checked_description(value: &str) -> Result<String, ApplicationError> {
    let value = value.trim().to_owned();
    if value.is_empty() {
        return Err(ApplicationError::InvalidInput {
            field: "description".into(),
            message: "tell the assistant what you want in a sentence or two".into(),
        });
    }
    if value.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(ApplicationError::InvalidInput {
            field: "description".into(),
            message: format!("must be {MAX_DESCRIPTION_CHARS} characters or fewer"),
        });
    }
    Ok(value)
}

/// The field list the model may fill for a record surface. Tasks are not a
/// plain-language drafting surface — they arrive through `propose_followup`.
fn draft_fields(kind: ProposalEntityType) -> Result<&'static str, ApplicationError> {
    match kind {
        ProposalEntityType::Contact => Ok(CONTACT_FIELDS),
        ProposalEntityType::Company => Ok(COMPANY_FIELDS),
        ProposalEntityType::Opportunity => Ok(OPPORTUNITY_FIELDS),
        ProposalEntityType::Task => Err(unsupported_task_draft()),
    }
}

/// Tasks are only drafted through `propose_followup`; every other drafting
/// entry point refuses them rather than guessing a shape.
fn unsupported_task_draft() -> ApplicationError {
    ApplicationError::InvalidInput {
        field: "entityType".into(),
        message: "tasks are drafted with the follow-up assistant, not this one".into(),
    }
}

fn create_prompt(kind: ProposalEntityType, description: &str) -> Result<String, ApplicationError> {
    let fields = draft_fields(kind)?;
    Ok(format!(
        "Draft a new {} record from this note.\n\nFields you may use:\n{fields}\n\nNote:\n{description}",
        kind.as_wire_value()
    ))
}

fn update_prompt(
    entity_type: ProposalEntityType,
    request: &str,
) -> Result<String, ApplicationError> {
    let fields = draft_fields(entity_type)?;
    Ok(format!(
        "Update this existing {} record. Include ONLY the fields that should \
         change; leave everything else out.\n\nFields you may use:\n{fields}\n\nRequested change:\n{request}",
        entity_type.as_wire_value()
    ))
}

/// A parsed model answer. Reading a field consumes it, so whatever is left at
/// the end is something ContractorCRM does not store and becomes a warning.
#[derive(Debug)]
struct Draft {
    values: serde_json::Map<String, serde_json::Value>,
    warnings: Vec<String>,
}

impl Draft {
    /// Parse the answer defensively: fences, leading prose, and trailing chatter
    /// are all tolerated; anything that is not a JSON object is a validation
    /// failure, never a panic.
    fn parse(text: &str) -> Result<Self, ApplicationError> {
        let unusable = || ApplicationError::ValidationFailed {
            code: "draft_unreadable",
            field: "description".into(),
            message: "The assistant's answer wasn't a usable draft. Try describing it again."
                .into(),
        };
        let start = text.find('{').ok_or_else(unusable)?;
        let end = text.rfind('}').ok_or_else(unusable)?;
        if end < start {
            return Err(unusable());
        }
        let value: serde_json::Value =
            serde_json::from_str(&text[start..=end]).map_err(|_| unusable())?;
        let values = value.as_object().ok_or_else(unusable)?.clone();
        Ok(Self {
            values,
            warnings: Vec::new(),
        })
    }

    /// A text field. Numbers are accepted (postal codes come back both ways);
    /// anything else is skipped with a warning.
    fn take_text(&mut self, key: &str) -> Option<String> {
        let value = self.values.remove(key)?;
        let text = match value {
            serde_json::Value::String(text) => text,
            serde_json::Value::Number(number) => number.to_string(),
            serde_json::Value::Null => return None,
            _ => {
                self.warn(format!(
                    "The assistant sent an unexpected value for \"{key}\" — it was left out."
                ));
                return None;
            }
        };
        let mut text = text.trim().to_owned();
        // The model controls this string. Validation caps some fields and not
        // others, so the draft side bounds every one of them and says so.
        let cap = draft_field_cap(key);
        if text.chars().count() > cap {
            text = text.chars().take(cap).collect();
            self.warn(format!(
                "The assistant's \"{key}\" was longer than ContractorCRM stores — it was \
                 shortened to {cap} characters."
            ));
        }
        (!text.is_empty()).then_some(text)
    }

    /// A whole-number field. Digit strings (with $ and separators) are parsed.
    fn take_integer(&mut self, key: &str) -> Option<i64> {
        let value = self.values.remove(key)?;
        match value {
            serde_json::Value::Number(number) => number.as_i64().or_else(|| {
                self.warn(format!(
                    "The assistant sent an unexpected number for \"{key}\" — it was left out."
                ));
                None
            }),
            serde_json::Value::String(text) => {
                let cleaned = text
                    .chars()
                    .filter(|character| character.is_ascii_digit() || *character == '-')
                    .collect::<String>();
                match cleaned.parse::<i64>() {
                    Ok(parsed) => Some(parsed),
                    Err(_) => {
                        self.warn(format!(
                            "The assistant sent an unexpected number for \"{key}\" — it was left out."
                        ));
                        None
                    }
                }
            }
            serde_json::Value::Null => None,
            _ => {
                self.warn(format!(
                    "The assistant sent an unexpected number for \"{key}\" — it was left out."
                ));
                None
            }
        }
    }

    fn warn(&mut self, warning: String) {
        if self.warnings.len() < MAX_WARNINGS {
            self.warnings.push(warning);
        }
    }

    /// Warn about everything the model sent that this app does not store.
    fn into_warnings(mut self) -> Vec<String> {
        let unknown = self.values.keys().cloned().collect::<Vec<_>>();
        for key in unknown {
            self.warn(format!(
                "The assistant suggested \"{key}\", which ContractorCRM doesn't store — it was ignored."
            ));
        }
        self.warnings
    }
}

/// Contact fields the model may set, on a create or an update.
fn apply_contact_draft_fields(draft: &mut Draft, patch: &mut ContactPatch) {
    set_if(&mut patch.first_name, draft.take_text("firstName"));
    set_if(&mut patch.last_name, draft.take_text("lastName"));
    set_if(&mut patch.display_name, draft.take_text("displayName"));
    set_if(&mut patch.role, draft.take_text("role"));
    if let Some(kind) = draft.take_text("kind") {
        patch.kind = kind;
    }
    if let Some(phone) = draft.take_text("phone") {
        set_channel(patch, "phone", phone);
    }
    if let Some(email) = draft.take_text("email") {
        set_channel(patch, "email", email);
    }
    set_if(
        &mut patch.preferred_contact_method,
        draft.take_text("preferredContactMethod"),
    );
    set_if(&mut patch.address_line1, draft.take_text("addressLine1"));
    set_if(&mut patch.address_line2, draft.take_text("addressLine2"));
    set_if(&mut patch.city, draft.take_text("city"));
    set_if(&mut patch.state, draft.take_text("state"));
    set_if(&mut patch.postal_code, draft.take_text("postalCode"));
    set_if(&mut patch.property_type, draft.take_text("propertyType"));
    set_if(&mut patch.notes, draft.take_text("notes"));
}

/// Create-only contact handling: the same fields plus a company looked up by
/// name, and a default kind so a bare note still produces a usable draft.
fn apply_contact_draft(
    storage: &Storage,
    draft: &mut Draft,
    patch: &mut ContactPatch,
) -> Result<(), ApplicationError> {
    apply_contact_draft_fields(draft, patch);
    if let Some(name) = draft.take_text("companyName") {
        match resolve_company(storage, &name)? {
            Some(company_id) => patch.company_id = Some(company_id),
            None => draft.warn(format!(
                "No company named \"{name}\" is on file — the contact isn't linked to one."
            )),
        }
    }
    if patch.kind.trim().is_empty() {
        patch.kind = "lead".into();
        draft.warn("Filed as a lead — change the kind if that's not right.".into());
    }
    // Show the display name the create would derive, so the diff matches the
    // record the user is about to get.
    if text(&patch.display_name).is_none() {
        let parts = [patch.first_name.as_deref(), patch.last_name.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        patch.display_name = non_empty(&parts);
    }
    Ok(())
}

fn apply_company_draft(draft: &mut Draft, patch: &mut CompanyPatch) {
    if let Some(name) = draft.take_text("name") {
        patch.name = name;
    }
    if let Some(kind) = draft.take_text("kind") {
        patch.kind = kind;
    }
    set_if(&mut patch.phone, draft.take_text("phone"));
    set_if(&mut patch.email, draft.take_text("email"));
    set_if(&mut patch.website, draft.take_text("website"));
    set_if(&mut patch.address_line1, draft.take_text("addressLine1"));
    set_if(&mut patch.address_line2, draft.take_text("addressLine2"));
    set_if(&mut patch.city, draft.take_text("city"));
    set_if(&mut patch.state, draft.take_text("state"));
    set_if(&mut patch.postal_code, draft.take_text("postalCode"));
    set_if(&mut patch.service_area, draft.take_text("serviceArea"));
    set_if(&mut patch.license_notes, draft.take_text("licenseNotes"));
    set_if(&mut patch.notes, draft.take_text("notes"));
    if patch.kind.trim().is_empty() {
        patch.kind = "lead".into();
        draft.warn("Filed as a lead — change the kind if that's not right.".into());
    }
}

fn apply_opportunity_draft_fields(draft: &mut Draft, patch: &mut OpportunityPatch) {
    if let Some(name) = draft.take_text("name") {
        patch.name = name;
    }
    if let Some(value_minor) = draft.take_integer("valueMinor") {
        patch.value_minor = value_minor;
    }
    if let Some(currency_code) = draft.take_text("currencyCode") {
        patch.currency_code = currency_code;
    }
    if let Some(probability) = draft.take_integer("probabilityPercent") {
        patch.probability_percent = Some(probability);
    }
    set_if(
        &mut patch.expected_close_date,
        draft.take_text("expectedCloseDate"),
    );
    set_if(&mut patch.source, draft.take_text("source"));
    set_if(&mut patch.source_label, draft.take_text("sourceLabel"));
    set_if(&mut patch.notes, draft.take_text("notes"));
    if patch.currency_code.trim().is_empty() {
        patch.currency_code = "USD".into();
    }
}

/// Create-only opportunity handling: the job has to hang off a contact or a
/// company, and the model may only name them — ids are resolved here.
fn apply_opportunity_draft(
    storage: &Storage,
    draft: &mut Draft,
    patch: &mut OpportunityPatch,
) -> Result<(), ApplicationError> {
    apply_opportunity_draft_fields(draft, patch);
    if let Some(name) = draft.take_text("contactName") {
        match resolve_contact(storage, &name)? {
            Some(contact_id) => patch.contact_id = Some(contact_id),
            None => draft.warn(format!("No contact named \"{name}\" is on file.")),
        }
    }
    if let Some(name) = draft.take_text("companyName") {
        match resolve_company(storage, &name)? {
            Some(company_id) => patch.company_id = Some(company_id),
            None => draft.warn(format!("No company named \"{name}\" is on file.")),
        }
    }
    Ok(())
}

fn set_if(target: &mut Option<String>, value: Option<String>) {
    if let Some(value) = value {
        *target = Some(value);
    }
}

/// Exact (case-insensitive) name match against active records. Ambiguous names
/// resolve to nothing — guessing which client is meant is not this app's job.
fn resolve_contact(storage: &Storage, name: &str) -> Result<Option<String>, ApplicationError> {
    resolve_by_name(
        storage,
        "SELECT id FROM contacts
         WHERE archived_at IS NULL AND lower(display_name) = lower(?1)
         LIMIT 2",
        name,
    )
}

fn resolve_company(storage: &Storage, name: &str) -> Result<Option<String>, ApplicationError> {
    resolve_by_name(
        storage,
        "SELECT id FROM companies
         WHERE archived_at IS NULL AND lower(name) = lower(?1)
         LIMIT 2",
        name,
    )
}

fn resolve_by_name(
    storage: &Storage,
    sql: &str,
    name: &str,
) -> Result<Option<String>, ApplicationError> {
    let mut statement = storage.connection().prepare(sql)?;
    let ids = statement
        .query_map([name.trim()], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(match ids.len() {
        1 => ids.into_iter().next(),
        _ => None,
    })
}

fn iso(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_draft_wrapped_in_prose_and_fences_still_parses() {
        let draft = Draft::parse("Sure! ```json\n{\"firstName\":\"Dana\"}\n``` Hope that helps.")
            .expect("json object found");
        assert_eq!(draft.values["firstName"], "Dana");
    }

    #[test]
    fn an_answer_with_no_json_object_is_a_validation_failure_not_a_panic() {
        let error = Draft::parse("I can't help with that.").expect_err("no object");
        assert_eq!(error.kind(), "validation_failed");
    }

    #[test]
    fn unexpected_value_shapes_become_warnings_and_are_left_out() {
        let mut draft = Draft::parse("{\"firstName\":{\"nope\":1},\"lastName\":\"Ruiz\"}")
            .expect("parse draft");
        assert_eq!(draft.take_text("firstName"), None);
        assert_eq!(draft.take_text("lastName"), Some("Ruiz".to_owned()));
        let warnings = draft.into_warnings();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("firstName"));
    }

    #[test]
    fn model_supplied_text_is_capped_per_field_with_a_warning() {
        let mut draft = Draft::parse(&format!(
            "{{\"notes\":\"{}\",\"city\":\"{}\"}}",
            "n".repeat(MAX_DRAFT_NOTE_CHARS + 50),
            "c".repeat(MAX_DRAFT_TEXT_CHARS + 50)
        ))
        .expect("parse draft");

        assert_eq!(
            draft
                .take_text("notes")
                .expect("notes kept")
                .chars()
                .count(),
            MAX_DRAFT_NOTE_CHARS
        );
        assert_eq!(
            draft.take_text("city").expect("city kept").chars().count(),
            MAX_DRAFT_TEXT_CHARS
        );
        let warnings = draft.into_warnings();
        assert_eq!(warnings.len(), 2);
        assert!(warnings.iter().any(|warning| warning.contains("notes")));
        assert!(warnings.iter().any(|warning| warning.contains("city")));
    }

    #[test]
    fn unknown_fields_are_reported_rather_than_silently_dropped() {
        let draft = Draft::parse("{\"favoriteColor\":\"blue\"}").expect("parse draft");
        let warnings = draft.into_warnings();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("favoriteColor"));
    }

    #[test]
    fn a_chatty_answer_never_returns_more_than_the_warning_cap() {
        let fields = (0..MAX_WARNINGS + 10)
            .map(|index| format!("\"madeUp{index}\":\"x\""))
            .collect::<Vec<_>>()
            .join(",");
        let draft = Draft::parse(&format!("{{{fields}}}")).expect("parse draft");
        assert_eq!(draft.into_warnings().len(), MAX_WARNINGS);
    }

    #[test]
    fn a_blank_or_over_long_description_is_refused_before_the_model_is_asked() {
        let blank = checked_description("   ").expect_err("a blank description is useless");
        assert_eq!(blank.kind(), "invalid_input");

        let over_long = checked_description(&"x".repeat(MAX_DESCRIPTION_CHARS + 1))
            .expect_err("an over-long description is refused");
        assert_eq!(over_long.kind(), "invalid_input");
        assert!(over_long
            .to_string()
            .contains(&MAX_DESCRIPTION_CHARS.to_string()));

        let accepted = checked_description(&format!("  {}  ", "x".repeat(MAX_DESCRIPTION_CHARS)))
            .expect("at the cap");
        assert_eq!(accepted.chars().count(), MAX_DESCRIPTION_CHARS);
    }

    #[test]
    fn the_diff_reports_only_fields_that_actually_change() {
        let before = CompanyPatch {
            name: "Coastal Fence".into(),
            kind: "client".into(),
            city: Some("Sanford".into()),
            ..CompanyPatch::default()
        };
        let mut after = before.clone();
        after.city = Some("Orlando".into());
        after.phone = Some("555-0100".into());

        let changes = diff(
            Some(&company_projection(&before)),
            &company_projection(&after),
        );
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].field, "phone");
        assert_eq!(changes[0].before, None);
        assert_eq!(changes[0].after.as_deref(), Some("555-0100"));
        assert_eq!(changes[1].field, "city");
        assert_eq!(changes[1].before.as_deref(), Some("Sanford"));
    }

    #[test]
    fn a_create_diff_has_no_before_values() {
        let patch = CompanyPatch {
            name: "Coastal Fence".into(),
            kind: "client".into(),
            ..CompanyPatch::default()
        };
        let changes = diff(None, &company_projection(&patch));
        assert!(changes.iter().all(|change| change.before.is_none()));
        assert_eq!(changes.len(), 2);
    }

    #[test]
    fn the_context_projection_is_bounded_and_leaves_out_empty_fields() {
        let patch = CompanyPatch {
            name: "Coastal Fence".into(),
            kind: "client".into(),
            notes: Some("x".repeat(500)),
            ..CompanyPatch::default()
        };
        let text = projection_text(&company_projection(&patch));
        assert!(text.starts_with("name: Coastal Fence\nkind: client\nnotes: "));
        assert!(!text.contains("website"));
        assert!(text.chars().count() < 400);
    }
}
