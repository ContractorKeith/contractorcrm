//! FTS5 projection coverage: migration backfill, transactional maintenance,
//! task-completion activities, query safety, filters, and rollback behavior.

use contractorcrm_lib::application::{
    archive_company, archive_contact, archive_opportunity, complete_task, create_company,
    create_contact, create_opportunity, create_task, delete_activity, log_activity, search_records,
    unarchive_company, unarchive_contact, unarchive_opportunity, update_activity, update_company,
    update_contact, update_opportunity, ActivityPatch, ArchiveRequest, CompanyPatch,
    CompleteTaskRequest, ContactPatch, CreateCompanyRequest, CreateContactRequest,
    CreateOpportunityRequest, CreateTaskRequest, DeleteActivityRequest, LogActivityRequest,
    OpportunityPatch, SearchResult, TaskPatch, UpdateActivityRequest, UpdateCompanyRequest,
    UpdateContactRequest, UpdateOpportunityRequest,
};
use contractorcrm_lib::domain::Actor;
use contractorcrm_lib::storage::Storage;
use rusqlite::Connection;

fn storage(temp: &tempfile::TempDir) -> Storage {
    Storage::open_in_app_data(temp.path()).expect("open storage")
}

fn hits(storage: &Storage, query: &str) -> Vec<SearchResult> {
    search_records(storage, query.into(), None, None).expect("search")
}

fn projection_count(storage: &Storage, entity_type: &str, entity_id: &str) -> i64 {
    storage
        .connection()
        .query_row(
            "SELECT count(*) FROM search_index WHERE entity_type = ?1 AND entity_id = ?2",
            [entity_type, entity_id],
            |row| row.get(0),
        )
        .expect("count search projection")
}

#[test]
fn migration_backfills_a_populated_v5_database() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("contractorcrm.sqlite3");
    let connection = Connection::open(&path).expect("open v5 fixture");
    connection.execute_batch(
        "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);
         INSERT INTO schema_migrations VALUES (1, 'x'), (2, 'x'), (3, 'x'), (4, 'x'), (5, 'x');
         CREATE TABLE companies (id TEXT PRIMARY KEY, name TEXT, phone TEXT, email TEXT, website TEXT, notes TEXT, archived_at TEXT);
         CREATE TABLE contacts (id TEXT PRIMARY KEY, display_name TEXT, notes TEXT, archived_at TEXT);
         CREATE TABLE contact_channels (contact_id TEXT, value TEXT);
         CREATE TABLE opportunities (id TEXT PRIMARY KEY, name TEXT, notes TEXT, source_label TEXT, archived_at TEXT);
         CREATE TABLE activities (id TEXT PRIMARY KEY, parent_type TEXT, parent_id TEXT, summary TEXT, body TEXT);
         INSERT INTO companies VALUES ('co', 'Backfill Builders', NULL, NULL, NULL, NULL, NULL);
         INSERT INTO contacts VALUES ('ct', 'Riley Backfill', NULL, NULL);
         INSERT INTO contacts VALUES ('archived-ct', 'Hidden Backfill', NULL, '2026-01-01T00:00:00Z');
         INSERT INTO contact_channels VALUES ('ct', 'riley@example.test');
         INSERT INTO opportunities VALUES ('op', 'Backfill deck', NULL, NULL, NULL);
         INSERT INTO activities VALUES ('ac', 'contact', 'ct', 'Backfill call', 'left a voicemail');
         INSERT INTO activities VALUES ('hidden-ac', 'contact', 'archived-ct', 'Hidden backfill call', NULL);",
    ).expect("seed v5 fixture");
    drop(connection);

    let storage = Storage::open(&path).expect("migrate fixture");
    let backfill_types = hits(&storage, "backfill")
        .into_iter()
        .map(|hit| hit.entity_type)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        backfill_types,
        ["activity", "company", "contact", "opportunity"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    );
    assert!(hits(&storage, "hidden").is_empty());
    assert_eq!(
        hits(&storage, "riley@example.test")[0].entity_type,
        "contact"
    );
}

#[test]
fn projections_follow_every_indexed_record_lifecycle() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = storage(&temp);
    let company = create_company(
        &mut storage,
        CreateCompanyRequest {
            actor: Actor::User,
            company: CompanyPatch {
                name: "Northstar Masonry".into(),
                kind: "client".into(),
                ..CompanyPatch::default()
            },
        },
    )
    .unwrap();
    assert_eq!(hits(&storage, "northstar")[0].entity_type, "company");
    let company = update_company(
        &mut storage,
        UpdateCompanyRequest {
            actor: Actor::User,
            company_id: company.id,
            expected_version: 1,
            patch: CompanyPatch {
                name: "Summit Masonry".into(),
                kind: "client".into(),
                ..CompanyPatch::default()
            },
        },
    )
    .unwrap();
    assert!(hits(&storage, "northstar").is_empty());
    let company = archive_company(
        &mut storage,
        ArchiveRequest {
            actor: Actor::User,
            id: company.id,
            expected_version: company.version,
        },
    )
    .unwrap();
    assert!(hits(&storage, "summit").is_empty());
    let company = unarchive_company(
        &mut storage,
        ArchiveRequest {
            actor: Actor::User,
            id: company.id,
            expected_version: company.version,
        },
    )
    .unwrap();
    assert_eq!(hits(&storage, "summit")[0].entity_type, "company");
    assert_eq!(projection_count(&storage, "company", &company.id), 1);

    let contact = create_contact(
        &mut storage,
        CreateContactRequest {
            actor: Actor::User,
            contact: ContactPatch {
                display_name: Some("Avery Stone".into()),
                kind: "lead".into(),
                channels: vec![contractorcrm_lib::application::ChannelInput {
                    kind: "email".into(),
                    label: None,
                    value: "avery@stone.test".into(),
                    preferred: true,
                }],
                ..ContactPatch::default()
            },
        },
    )
    .unwrap();
    assert_eq!(hits(&storage, "avery@stone.test")[0].entity_type, "contact");
    let contact = update_contact(
        &mut storage,
        UpdateContactRequest {
            actor: Actor::User,
            contact_id: contact.id,
            expected_version: 1,
            patch: ContactPatch {
                display_name: Some("Morgan Stone".into()),
                kind: "lead".into(),
                ..ContactPatch::default()
            },
        },
    )
    .unwrap();
    assert!(hits(&storage, "avery").is_empty());
    let contact = archive_contact(
        &mut storage,
        ArchiveRequest {
            actor: Actor::User,
            id: contact.id,
            expected_version: contact.version,
        },
    )
    .unwrap();
    assert!(hits(&storage, "morgan").is_empty());
    let mut contact = unarchive_contact(
        &mut storage,
        ArchiveRequest {
            actor: Actor::User,
            id: contact.id,
            expected_version: contact.version,
        },
    )
    .unwrap();
    assert_eq!(hits(&storage, "morgan")[0].entity_type, "contact");
    assert_eq!(projection_count(&storage, "contact", &contact.id), 1);

    let opportunity = create_opportunity(
        &mut storage,
        CreateOpportunityRequest {
            actor: Actor::User,
            stage_id: None,
            opportunity: OpportunityPatch {
                name: "Stone patio".into(),
                contact_id: Some(contact.id.clone()),
                currency_code: "USD".into(),
                ..OpportunityPatch::default()
            },
        },
    )
    .unwrap();
    assert_eq!(hits(&storage, "patio")[0].entity_type, "opportunity");
    let opportunity = update_opportunity(
        &mut storage,
        UpdateOpportunityRequest {
            actor: Actor::User,
            opportunity_id: opportunity.id,
            expected_version: 1,
            patch: OpportunityPatch {
                name: "Stone steps".into(),
                contact_id: Some(contact.id.clone()),
                currency_code: "USD".into(),
                ..OpportunityPatch::default()
            },
        },
    )
    .unwrap();
    assert!(hits(&storage, "patio").is_empty());
    let opportunity = archive_opportunity(
        &mut storage,
        ArchiveRequest {
            actor: Actor::User,
            id: opportunity.id,
            expected_version: opportunity.version,
        },
    )
    .unwrap();
    assert!(hits(&storage, "steps").is_empty());
    let opportunity = unarchive_opportunity(
        &mut storage,
        ArchiveRequest {
            actor: Actor::User,
            id: opportunity.id,
            expected_version: opportunity.version,
        },
    )
    .unwrap();
    assert_eq!(hits(&storage, "steps")[0].entity_type, "opportunity");
    assert_eq!(
        projection_count(&storage, "opportunity", &opportunity.id),
        1
    );

    let activity = log_activity(
        &mut storage,
        LogActivityRequest {
            actor: Actor::User,
            parent_type: "contact".into(),
            parent_id: contact.id.clone(),
            activity: ActivityPatch {
                kind: "note".into(),
                summary: "Measure stone landing".into(),
                body: Some("Confirm tread depth".into()),
                ..ActivityPatch::default()
            },
        },
    )
    .unwrap();
    assert_eq!(hits(&storage, "tread")[0].entity_type, "activity");
    let activity_hits = hits(&storage, "tread");
    let activity_hit = &activity_hits[0];
    assert_eq!(activity_hit.parent_type.as_deref(), Some("contact"));
    assert_eq!(activity_hit.parent_id.as_deref(), Some(contact.id.as_str()));
    contact = archive_contact(
        &mut storage,
        ArchiveRequest {
            actor: Actor::User,
            id: contact.id,
            expected_version: contact.version,
        },
    )
    .unwrap();
    assert!(hits(&storage, "tread").is_empty());
    let _unarchived_contact = unarchive_contact(
        &mut storage,
        ArchiveRequest {
            actor: Actor::User,
            id: contact.id,
            expected_version: contact.version,
        },
    )
    .unwrap();
    assert_eq!(hits(&storage, "tread").len(), 1);
    let activity = update_activity(
        &mut storage,
        UpdateActivityRequest {
            actor: Actor::User,
            activity_id: activity.id,
            expected_version: 1,
            patch: ActivityPatch {
                kind: "note".into(),
                summary: "Measure landing".into(),
                body: Some("Confirm riser height".into()),
                ..ActivityPatch::default()
            },
        },
    )
    .unwrap();
    assert!(hits(&storage, "tread").is_empty());
    delete_activity(
        &mut storage,
        DeleteActivityRequest {
            actor: Actor::User,
            activity_id: activity.id.clone(),
            expected_version: activity.version,
        },
    )
    .unwrap();
    assert!(hits(&storage, "riser").is_empty());
    assert_eq!(projection_count(&storage, "activity", &activity.id), 0);
}

#[test]
fn task_completion_activity_and_safe_filters_are_searchable() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = storage(&temp);
    let contact = create_contact(
        &mut storage,
        CreateContactRequest {
            actor: Actor::User,
            contact: ContactPatch {
                display_name: Some("Casey Filter".into()),
                kind: "lead".into(),
                ..ContactPatch::default()
            },
        },
    )
    .unwrap();
    let task = create_task(
        &mut storage,
        CreateTaskRequest {
            actor: Actor::User,
            task: TaskPatch {
                title: "Call about warranty".into(),
                parent_type: Some("contact".into()),
                parent_id: Some(contact.id.clone()),
                ..TaskPatch::default()
            },
        },
    )
    .unwrap();
    complete_task(
        &mut storage,
        CompleteTaskRequest {
            actor: Actor::User,
            task_id: task.id,
            expected_version: 1,
            log_activity: true,
        },
    )
    .unwrap();
    assert_eq!(
        hits(&storage, "completed warranty")[0].entity_type,
        "activity"
    );
    assert!(search_records(&storage, "".into(), None, None)
        .unwrap()
        .is_empty());
    assert!(search_records(&storage, "*** OR *".into(), None, None)
        .unwrap()
        .is_empty());
    assert_eq!(
        search_records(
            &storage,
            "casey".into(),
            Some(vec!["contact".into()]),
            Some(999)
        )
        .unwrap()
        .len(),
        1
    );
    assert_eq!(
        search_records(
            &storage,
            "casey".into(),
            Some(vec!["contact".into(), "contact".into()]),
            None,
        )
        .unwrap()
        .len(),
        1
    );
    assert!(search_records(
        &storage,
        "casey".into(),
        Some(vec!["contact".into(); 5]),
        None,
    )
    .is_err());
    assert!(search_records(&storage, "casey".into(), Some(vec!["bad".into()]), None).is_err());
}

#[test]
fn failed_write_rolls_back_the_projection() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = storage(&temp);
    let contact = create_contact(
        &mut storage,
        CreateContactRequest {
            actor: Actor::User,
            contact: ContactPatch {
                display_name: Some("Rollback Robin".into()),
                kind: "lead".into(),
                ..ContactPatch::default()
            },
        },
    )
    .unwrap();
    storage
        .connection()
        .execute_batch(
            "CREATE TRIGGER force_command_log_failure
             BEFORE INSERT ON command_log
             BEGIN
               SELECT RAISE(ABORT, 'forced command log failure');
             END;",
        )
        .expect("install failure trigger");
    let result = update_contact(
        &mut storage,
        UpdateContactRequest {
            actor: Actor::User,
            contact_id: contact.id.clone(),
            expected_version: 1,
            patch: ContactPatch {
                display_name: Some("Wrong Result".into()),
                kind: "lead".into(),
                ..ContactPatch::default()
            },
        },
    );
    assert!(result.is_err());
    assert_eq!(hits(&storage, "rollback")[0].title, "Rollback Robin");
    assert!(hits(&storage, "wrong").is_empty());
    assert_eq!(projection_count(&storage, "contact", &contact.id), 1);
    let stored_name: String = storage
        .connection()
        .query_row(
            "SELECT display_name FROM contacts WHERE id = ?1",
            [&contact.id],
            |row| row.get(0),
        )
        .expect("load rolled-back contact");
    assert_eq!(stored_name, "Rollback Robin");
}
