//! Integration tests for AI explanations of deterministic attention flags:
//! the bounded projection, the disabled and stale-flag invariants, and flag id
//! stability. No network and no real keychain are involved — the provider is a
//! canned one and credentials live in memory.

use std::sync::Mutex;

use chrono::{Duration, SecondsFormat, Utc};
use contractorcrm_lib::ai::{
    set_ai_settings, CompletionProvider, InMemoryCredentialStore, ProviderCheck,
    ProviderCompletion, ProviderRequest, SetAiSettingsRequest,
};
use contractorcrm_lib::application::{
    create_contact, create_task, get_attention_flags, log_activity, ActivityPatch, ContactPatch,
    CreateContactRequest, CreateTaskRequest, LogActivityRequest, TaskPatch,
};
use contractorcrm_lib::domain::{Actor, Contact};
use contractorcrm_lib::error::ApplicationError;
use contractorcrm_lib::explain::plan_explanation;
use contractorcrm_lib::storage::Storage;

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
        unreachable!("explanations never run a connection check")
    }
}

fn open_storage(temp: &tempfile::TempDir) -> Storage {
    Storage::open_in_app_data(temp.path()).expect("open storage")
}

/// UTC ISO-8601 timestamp this many days away from now (negative = past).
fn days_from_now(days: i64) -> String {
    (Utc::now() + Duration::days(days)).to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn make_lead(storage: &mut Storage, display_name: &str) -> Contact {
    create_contact(
        storage,
        CreateContactRequest {
            actor: Actor::User,
            contact: ContactPatch {
                display_name: Some(display_name.into()),
                kind: "lead".into(),
                ..ContactPatch::default()
            },
        },
    )
    .expect("create lead")
}

fn log_touch(storage: &mut Storage, contact_id: &str, occurred_at: &str) {
    log_activity(
        storage,
        LogActivityRequest {
            actor: Actor::User,
            parent_type: "contact".into(),
            parent_id: contact_id.into(),
            activity: ActivityPatch {
                kind: "call".into(),
                direction: Some("outbound".into()),
                occurred_at: Some(occurred_at.into()),
                summary: "Phone call".into(),
                ..ActivityPatch::default()
            },
        },
    )
    .expect("log activity");
}

fn turn_the_assistant_on(storage: &mut Storage, credentials: &InMemoryCredentialStore) {
    set_ai_settings(
        storage,
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

/// Seed two stale leads (so the projection has something to leak) and return
/// the storage plus the first lead.
fn seeded_stale_leads(storage: &mut Storage) -> (Contact, Contact) {
    let stale = make_lead(storage, "Stale Sam");
    log_touch(storage, &stale.id, &days_from_now(-40));
    let other = make_lead(storage, "Marco Silva");
    log_touch(storage, &other.id, &days_from_now(-45));
    (stale, other)
}

#[test]
fn explaining_a_stale_lead_flag_round_trips_through_the_provider() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let credentials = InMemoryCredentialStore::with_key("sk-test");
    let (stale, _other) = seeded_stale_leads(&mut storage);
    turn_the_assistant_on(&mut storage, &credentials);

    let flag_id = format!("stale_lead:{}", stale.id);
    let plan =
        plan_explanation(&storage, &credentials, &flag_id, None).expect("plan the explanation");
    let provider = CannedProvider::new("Sam has gone quiet for 40 days. Call him this week.");
    let explanation = plan.run_with(&provider).expect("run the explanation");

    assert_eq!(explanation.flag_id, flag_id);
    assert_eq!(explanation.endpoint_host, "127.0.0.1:11434");
    assert!(explanation.local, "a local endpoint keeps data on the box");
    assert_eq!(explanation.explanation.model, "canned-model");
    assert_eq!(
        explanation.explanation.text,
        "Sam has gone quiet for 40 days. Call him this week."
    );
    assert_eq!(explanation.explanation.purpose, "explain_attention_flag");

    // The disclosure list names the flagged lead and nobody else.
    let refs = &explanation.explanation.included_record_refs;
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].entity_type, "contact");
    assert_eq!(refs[0].entity_id, stale.id);
    assert_eq!(refs[0].label, "Stale Sam");

    // Exactly one call was made, with the projection we planned.
    let seen = provider.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].purpose, "explain_attention_flag");
    assert!(seen[0].system_text.contains("never invent facts"));
}

#[test]
fn the_projection_carries_the_rule_facts_and_no_other_contact() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let credentials = InMemoryCredentialStore::new();
    let (stale, other) = seeded_stale_leads(&mut storage);
    turn_the_assistant_on(&mut storage, &credentials);

    let plan = plan_explanation(
        &storage,
        &credentials,
        &format!("stale_lead:{}", stale.id),
        None,
    )
    .expect("plan the explanation");
    let context = plan
        .request()
        .context_text
        .clone()
        .expect("the request carries a projection");

    assert!(context.contains("Rule: stale lead"), "{context}");
    assert!(context.contains("Threshold: 21 day(s)"), "{context}");
    assert!(context.contains("Last activity: "), "{context}");
    assert!(context.contains("Today: "), "{context}");
    assert!(context.contains("Stale Sam"), "{context}");
    assert!(context.contains(&stale.id), "{context}");
    assert!(
        context.contains("Deterministic finding: "),
        "the model explains the rule's own words: {context}"
    );

    // Nothing about the other lead — or any credential — leaves the machine.
    assert!(!context.contains("Marco Silva"), "{context}");
    assert!(!context.contains(&other.id), "{context}");
    assert!(!context.contains("sk-"), "{context}");
}

#[test]
fn an_overdue_task_projection_names_the_task_and_its_due_date() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let credentials = InMemoryCredentialStore::new();
    let task = create_task(
        &mut storage,
        CreateTaskRequest {
            actor: Actor::User,
            task: TaskPatch {
                title: "Call the county office".into(),
                due_at: Some(days_from_now(-3)),
                ..TaskPatch::default()
            },
        },
    )
    .expect("create task");
    turn_the_assistant_on(&mut storage, &credentials);

    let plan = plan_explanation(
        &storage,
        &credentials,
        &format!("overdue_task:{}", task.id),
        None,
    )
    .expect("plan the explanation");
    let context = plan
        .request()
        .context_text
        .clone()
        .expect("the request carries a projection");

    assert!(context.contains("Rule: overdue task"), "{context}");
    assert!(
        context.contains("Task: Call the county office"),
        "{context}"
    );
    assert!(context.contains("Due: "), "{context}");
    assert!(context.contains(&task.id), "{context}");
}

#[test]
fn an_unknown_or_stale_flag_id_is_not_found() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let credentials = InMemoryCredentialStore::new();
    let fresh = make_lead(&mut storage, "Fresh Fran");
    log_touch(&mut storage, &fresh.id, &days_from_now(0));
    turn_the_assistant_on(&mut storage, &credentials);

    // A flag that never existed.
    let error = plan_explanation(&storage, &credentials, "stale_lead:nope", None)
        .expect_err("unknown flag id");
    assert_eq!(error.kind(), "not_found");

    // A record that exists but is not currently flagged — the stale screen the
    // user clicked from has moved on.
    let error = plan_explanation(
        &storage,
        &credentials,
        &format!("stale_lead:{}", fresh.id),
        None,
    )
    .expect_err("flag no longer current");
    assert_eq!(error.kind(), "not_found");
}

#[test]
fn a_switched_off_assistant_explains_nothing_and_reads_no_credentials() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let credentials = InMemoryCredentialStore::with_key("sk-test");
    let (stale, _other) = seeded_stale_leads(&mut storage);

    // Never configured at all.
    let error = plan_explanation(
        &storage,
        &credentials,
        &format!("stale_lead:{}", stale.id),
        None,
    )
    .expect_err("assistant is off");
    assert_eq!(error.kind(), "provider_unavailable");
    assert!(error.to_string().contains("off"), "{error}");

    // Explicitly turned off.
    set_ai_settings(
        &mut storage,
        &credentials,
        SetAiSettingsRequest {
            actor: Actor::User,
            enabled: false,
            provider_label: "Local model".into(),
            base_url: "http://127.0.0.1:11434/v1".into(),
            model: "llama3.1".into(),
        },
    )
    .expect("save disabled settings");
    let error = plan_explanation(
        &storage,
        &credentials,
        &format!("stale_lead:{}", stale.id),
        None,
    )
    .expect_err("assistant is off");
    assert_eq!(error.kind(), "provider_unavailable");

    assert_eq!(
        credentials.read_count(),
        0,
        "a disabled assistant must never read the credential store"
    );
}

#[test]
fn flag_ids_stay_stable_across_evaluations_so_explain_can_reference_them() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let credentials = InMemoryCredentialStore::new();
    let (stale, _other) = seeded_stale_leads(&mut storage);
    turn_the_assistant_on(&mut storage, &credentials);

    let first = get_attention_flags(&storage, None).expect("first evaluation");
    let second = get_attention_flags(&storage, None).expect("second evaluation");
    let ids = |flags: &[contractorcrm_lib::attention::AttentionFlag]| {
        flags.iter().map(|flag| flag.id.clone()).collect::<Vec<_>>()
    };
    assert_eq!(ids(&first), ids(&second));
    assert!(ids(&first).contains(&format!("stale_lead:{}", stale.id)));

    // And the id the UI holds still resolves to the same flag.
    let plan = plan_explanation(
        &storage,
        &credentials,
        &format!("stale_lead:{}", stale.id),
        None,
    )
    .expect("plan the explanation");
    let explanation = plan
        .run_with(&CannedProvider::new("Give Sam a call."))
        .expect("run the explanation");
    assert_eq!(explanation.flag_id, format!("stale_lead:{}", stale.id));
}

#[test]
fn a_blank_flag_id_is_a_caller_error() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let credentials = InMemoryCredentialStore::new();
    turn_the_assistant_on(&mut storage, &credentials);

    let error = plan_explanation(&storage, &credentials, "  ", None).expect_err("blank flag id");
    assert_eq!(error.kind(), "invalid_input");
}
