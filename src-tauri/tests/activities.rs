//! Integration tests for the activities domain — logging on each parent type,
//! parent validation, the unified newest-first timeline with linked-opportunity
//! projection, version conflicts, hard delete, and command_log rows.

use contractorcrm_lib::application::{
    create_company, create_contact, create_opportunity, delete_activity, get_timeline,
    log_activity, update_activity, ActivityPatch, CompanyPatch, ContactPatch, CreateCompanyRequest,
    CreateContactRequest, CreateOpportunityRequest, DeleteActivityRequest, LogActivityRequest,
    OpportunityPatch, UpdateActivityRequest,
};
use contractorcrm_lib::domain::{
    Activity, ActivityDirection, ActivityKind, Actor, Company, Contact, Opportunity, ParentType,
};
use contractorcrm_lib::error::ApplicationError;
use contractorcrm_lib::storage::Storage;

fn open_storage(temp: &tempfile::TempDir) -> Storage {
    Storage::open_in_app_data(temp.path()).expect("open storage")
}

fn make_contact(storage: &mut Storage, display_name: &str) -> Contact {
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
    .expect("create contact")
}

fn make_company(storage: &mut Storage, name: &str) -> Company {
    create_company(
        storage,
        CreateCompanyRequest {
            actor: Actor::User,
            company: CompanyPatch {
                name: name.into(),
                kind: "client".into(),
                ..CompanyPatch::default()
            },
        },
    )
    .expect("create company")
}

fn make_opportunity(
    storage: &mut Storage,
    name: &str,
    contact_id: Option<&str>,
    company_id: Option<&str>,
) -> Opportunity {
    create_opportunity(
        storage,
        CreateOpportunityRequest {
            actor: Actor::User,
            stage_id: None,
            opportunity: OpportunityPatch {
                name: name.into(),
                contact_id: contact_id.map(Into::into),
                company_id: company_id.map(Into::into),
                currency_code: "USD".into(),
                value_minor: 100_000,
                ..OpportunityPatch::default()
            },
        },
    )
    .expect("create opportunity")
}

fn log_on(
    storage: &mut Storage,
    parent_type: &str,
    parent_id: &str,
    summary: &str,
    occurred_at: Option<&str>,
) -> Activity {
    log_activity(
        storage,
        LogActivityRequest {
            actor: Actor::User,
            parent_type: parent_type.into(),
            parent_id: parent_id.into(),
            activity: ActivityPatch {
                kind: "call".into(),
                direction: Some("outbound".into()),
                occurred_at: occurred_at.map(Into::into),
                summary: summary.into(),
                body: None,
            },
        },
    )
    .expect("log activity")
}

fn command_log_summaries(storage: &Storage, entity_id: &str) -> Vec<String> {
    let mut statement = storage
        .connection()
        .prepare(
            "SELECT summary FROM command_log
             WHERE entity_type = 'activity' AND entity_id = ?1 ORDER BY created_at, id",
        )
        .expect("prepare command_log query");
    statement
        .query_map([entity_id], |row| row.get(0))
        .expect("query command_log")
        .collect::<rusqlite::Result<Vec<String>>>()
        .expect("collect command_log summaries")
}

// ---------------------------------------------------------------------------
// Logging on each parent type + validation
// ---------------------------------------------------------------------------

#[test]
fn logs_an_activity_on_each_parent_type() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Homeowner");
    let company = make_company(&mut storage, "Ridgeline Builders");
    let opportunity = make_opportunity(&mut storage, "Pool fence", Some(&contact.id), None);

    let on_contact = log_on(&mut storage, "contact", &contact.id, "Called Dana", None);
    assert_eq!(on_contact.parent_type, ParentType::Contact);
    assert_eq!(on_contact.parent_id, contact.id);
    assert_eq!(on_contact.kind, ActivityKind::Call);
    assert_eq!(on_contact.direction, ActivityDirection::Outbound);
    assert_eq!(on_contact.version, 1);

    let on_company = log_on(&mut storage, "company", &company.id, "Emailed office", None);
    assert_eq!(on_company.parent_type, ParentType::Company);

    let on_opportunity = log_on(
        &mut storage,
        "opportunity",
        &opportunity.id,
        "Site visit",
        None,
    );
    assert_eq!(on_opportunity.parent_type, ParentType::Opportunity);

    // A note with no direction defaults to none.
    let note = log_activity(
        &mut storage,
        LogActivityRequest {
            actor: Actor::Agent,
            parent_type: "contact".into(),
            parent_id: contact.id.clone(),
            activity: ActivityPatch {
                kind: "note".into(),
                direction: None,
                occurred_at: None,
                summary: "Prefers texts".into(),
                body: Some("Reach out **after 5pm**.".into()),
            },
        },
    )
    .expect("log note");
    assert_eq!(note.direction, ActivityDirection::None);
    assert_eq!(note.actor, Actor::Agent);
    assert_eq!(note.body.as_deref(), Some("Reach out **after 5pm**."));
}

#[test]
fn logging_on_a_missing_parent_is_not_found() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);

    let error = log_activity(
        &mut storage,
        LogActivityRequest {
            actor: Actor::User,
            parent_type: "contact".into(),
            parent_id: "missing-contact".into(),
            activity: ActivityPatch {
                kind: "call".into(),
                summary: "Called nobody".into(),
                ..ActivityPatch::default()
            },
        },
    )
    .expect_err("missing parent must fail");
    assert_eq!(error.kind(), "not_found");
}

#[test]
fn empty_summary_and_bad_enums_are_rejected() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Homeowner");

    let blank_summary = log_activity(
        &mut storage,
        LogActivityRequest {
            actor: Actor::User,
            parent_type: "contact".into(),
            parent_id: contact.id.clone(),
            activity: ActivityPatch {
                kind: "call".into(),
                summary: "   ".into(),
                ..ActivityPatch::default()
            },
        },
    )
    .expect_err("blank summary must fail");
    assert_eq!(blank_summary.kind(), "invalid_input");

    let bad_kind = log_activity(
        &mut storage,
        LogActivityRequest {
            actor: Actor::User,
            parent_type: "contact".into(),
            parent_id: contact.id.clone(),
            activity: ActivityPatch {
                kind: "fax".into(),
                summary: "Faxed?".into(),
                ..ActivityPatch::default()
            },
        },
    )
    .expect_err("unknown kind must fail");
    assert_eq!(bad_kind.kind(), "invalid_input");

    // Notes carry no direction.
    let directed_note = log_activity(
        &mut storage,
        LogActivityRequest {
            actor: Actor::User,
            parent_type: "contact".into(),
            parent_id: contact.id.clone(),
            activity: ActivityPatch {
                kind: "note".into(),
                direction: Some("inbound".into()),
                summary: "Note with direction".into(),
                ..ActivityPatch::default()
            },
        },
    )
    .expect_err("directed note must fail");
    assert_eq!(directed_note.kind(), "invalid_input");
}

// ---------------------------------------------------------------------------
// Timeline ordering and projections
// ---------------------------------------------------------------------------

#[test]
fn timeline_is_newest_first_by_occurred_at_not_created_at() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Homeowner");

    // Logged out of order on purpose: yesterday's call goes in last.
    log_on(
        &mut storage,
        "contact",
        &contact.id,
        "Middle",
        Some("2026-08-13T15:00:00.000Z"),
    );
    log_on(
        &mut storage,
        "contact",
        &contact.id,
        "Newest",
        Some("2026-08-14T09:00:00.000Z"),
    );
    log_on(
        &mut storage,
        "contact",
        &contact.id,
        "Oldest",
        Some("2026-08-12T08:00:00.000Z"),
    );

    let timeline = get_timeline(&storage, "contact", &contact.id, false).expect("timeline");
    let summaries: Vec<&str> = timeline.iter().map(|a| a.summary.as_str()).collect();
    assert_eq!(summaries, vec!["Newest", "Middle", "Oldest"]);
}

#[test]
fn contact_timeline_includes_linked_opportunity_activities_only_when_asked() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Homeowner");
    let other = make_contact(&mut storage, "Sam Neighbor");
    let linked = make_opportunity(&mut storage, "Pool fence", Some(&contact.id), None);
    let unrelated = make_opportunity(&mut storage, "Other job", Some(&other.id), None);

    log_on(
        &mut storage,
        "contact",
        &contact.id,
        "Direct call",
        Some("2026-08-14T10:00:00.000Z"),
    );
    log_on(
        &mut storage,
        "opportunity",
        &linked.id,
        "Opportunity visit",
        Some("2026-08-14T12:00:00.000Z"),
    );
    log_on(
        &mut storage,
        "opportunity",
        &unrelated.id,
        "Someone else's job",
        Some("2026-08-14T13:00:00.000Z"),
    );

    // Without the projection, only the contact's own activity shows.
    let own_only = get_timeline(&storage, "contact", &contact.id, false).expect("own timeline");
    let summaries: Vec<&str> = own_only.iter().map(|a| a.summary.as_str()).collect();
    assert_eq!(summaries, vec!["Direct call"]);

    // With it, linked-opportunity activities join in, newest first; the
    // unrelated opportunity stays out.
    let related = get_timeline(&storage, "contact", &contact.id, true).expect("related timeline");
    let summaries: Vec<&str> = related.iter().map(|a| a.summary.as_str()).collect();
    assert_eq!(summaries, vec!["Opportunity visit", "Direct call"]);
    assert_eq!(related[0].parent_type, ParentType::Opportunity);
    assert_eq!(related[0].parent_id, linked.id);
}

#[test]
fn company_timeline_projects_activities_of_company_linked_opportunities() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let company = make_company(&mut storage, "Ridgeline Builders");
    let other = make_contact(&mut storage, "Sam Neighbor");
    let linked = make_opportunity(&mut storage, "HOA perimeter", None, Some(&company.id));
    let unrelated = make_opportunity(&mut storage, "Other job", Some(&other.id), None);

    log_on(
        &mut storage,
        "company",
        &company.id,
        "Office email",
        Some("2026-08-14T08:00:00.000Z"),
    );
    log_on(
        &mut storage,
        "opportunity",
        &linked.id,
        "Walked the perimeter",
        Some("2026-08-14T11:00:00.000Z"),
    );
    log_on(
        &mut storage,
        "opportunity",
        &unrelated.id,
        "Unrelated visit",
        Some("2026-08-14T12:00:00.000Z"),
    );

    let related = get_timeline(&storage, "company", &company.id, true).expect("related timeline");
    let summaries: Vec<&str> = related.iter().map(|a| a.summary.as_str()).collect();
    assert_eq!(summaries, vec!["Walked the perimeter", "Office email"]);

    let own_only = get_timeline(&storage, "company", &company.id, false).expect("own timeline");
    assert_eq!(own_only.len(), 1);
}

#[test]
fn timeline_for_a_missing_parent_is_not_found() {
    let temp = tempfile::tempdir().expect("tempdir");
    let storage = open_storage(&temp);

    let error =
        get_timeline(&storage, "opportunity", "missing", false).expect_err("must be not_found");
    assert_eq!(error.kind(), "not_found");
}

// ---------------------------------------------------------------------------
// Update, delete, versions, and the command log
// ---------------------------------------------------------------------------

#[test]
fn update_bumps_the_version_and_stale_versions_conflict() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Homeowner");
    let activity = log_on(&mut storage, "contact", &contact.id, "First pass", None);

    let updated = update_activity(
        &mut storage,
        UpdateActivityRequest {
            actor: Actor::User,
            activity_id: activity.id.clone(),
            expected_version: 1,
            patch: ActivityPatch {
                kind: "meeting".into(),
                direction: None,
                occurred_at: Some("2026-08-13T16:00:00.000Z".into()),
                summary: "Kickoff meeting".into(),
                body: Some("Discussed scope.".into()),
            },
        },
    )
    .expect("update activity");
    assert_eq!(updated.kind, ActivityKind::Meeting);
    assert_eq!(updated.summary, "Kickoff meeting");
    assert_eq!(updated.occurred_at, "2026-08-13T16:00:00.000Z");
    assert_eq!(updated.version, 2);
    // The parent and original actor never change on update.
    assert_eq!(updated.parent_id, contact.id);
    assert_eq!(updated.actor, activity.actor);

    let stale = update_activity(
        &mut storage,
        UpdateActivityRequest {
            actor: Actor::User,
            activity_id: activity.id.clone(),
            expected_version: 1, // stale — current is 2
            patch: ActivityPatch {
                kind: "note".into(),
                summary: "Too late".into(),
                ..ActivityPatch::default()
            },
        },
    )
    .expect_err("stale version must conflict");
    assert!(matches!(
        stale,
        ApplicationError::VersionConflict {
            expected: 1,
            current: 2,
            ..
        }
    ));
    assert_eq!(stale.kind(), "version_conflict");
}

#[test]
fn delete_removes_the_activity_and_logs_the_command() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Homeowner");
    let activity = log_on(&mut storage, "contact", &contact.id, "Wrong record", None);

    // Stale version is rejected before anything is deleted.
    let stale = delete_activity(
        &mut storage,
        DeleteActivityRequest {
            actor: Actor::User,
            activity_id: activity.id.clone(),
            expected_version: 9,
        },
    )
    .expect_err("stale delete must conflict");
    assert_eq!(stale.kind(), "version_conflict");

    delete_activity(
        &mut storage,
        DeleteActivityRequest {
            actor: Actor::User,
            activity_id: activity.id.clone(),
            expected_version: 1,
        },
    )
    .expect("delete activity");

    let timeline = get_timeline(&storage, "contact", &contact.id, false).expect("timeline");
    assert!(timeline.is_empty(), "deleted activity must be gone");

    let missing = delete_activity(
        &mut storage,
        DeleteActivityRequest {
            actor: Actor::User,
            activity_id: activity.id.clone(),
            expected_version: 1,
        },
    )
    .expect_err("second delete must be not_found");
    assert_eq!(missing.kind(), "not_found");
}

#[test]
fn every_mutation_writes_a_command_log_row() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Homeowner");
    let activity = log_on(&mut storage, "contact", &contact.id, "Logged call", None);

    update_activity(
        &mut storage,
        UpdateActivityRequest {
            actor: Actor::Agent,
            activity_id: activity.id.clone(),
            expected_version: 1,
            patch: ActivityPatch {
                kind: "call".into(),
                direction: Some("inbound".into()),
                summary: "Return call".into(),
                ..ActivityPatch::default()
            },
        },
    )
    .expect("update activity");
    delete_activity(
        &mut storage,
        DeleteActivityRequest {
            actor: Actor::User,
            activity_id: activity.id.clone(),
            expected_version: 2,
        },
    )
    .expect("delete activity");

    let summaries = command_log_summaries(&storage, &activity.id);
    assert_eq!(
        summaries,
        vec![
            "logged call activity \"Logged call\"",
            "updated activity \"Return call\"",
            "deleted activity \"Return call\"",
        ]
    );
}
