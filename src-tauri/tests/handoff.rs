//! Integration tests for quote/job hand-off references and the versioned
//! envelope export — link/unlink round-trips, the won-stage rule for jobs,
//! version conflicts, envelope JSON shape, and migration 003 behavior.

use contractorcrm_lib::application::{
    create_contact, create_opportunity, export_handoff_envelope, get_opportunity, link_job,
    link_quote, list_stages, move_opportunity_stage, unlink_quote, ChannelInput, ContactPatch,
    CreateContactRequest, CreateOpportunityRequest, HandoffRefInput, LinkJobRequest,
    LinkQuoteRequest, MoveOpportunityStageRequest, OpportunityPatch, UnlinkHandoffRequest,
};
use contractorcrm_lib::domain::{Actor, Contact, Opportunity, StageKind};
use contractorcrm_lib::error::ApplicationError;
use contractorcrm_lib::storage::{latest_migration_version, Storage};

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
                channels: vec![
                    ChannelInput {
                        kind: "phone".into(),
                        label: Some("mobile".into()),
                        value: "555-0100".into(),
                        preferred: true,
                    },
                    ChannelInput {
                        kind: "email".into(),
                        label: None,
                        value: "dana@example.com".into(),
                        preferred: false,
                    },
                ],
                ..ContactPatch::default()
            },
        },
    )
    .expect("create contact")
}

fn make_opportunity(storage: &mut Storage, name: &str, contact_id: &str) -> Opportunity {
    create_opportunity(
        storage,
        CreateOpportunityRequest {
            actor: Actor::User,
            stage_id: None,
            opportunity: OpportunityPatch {
                name: name.into(),
                contact_id: Some(contact_id.into()),
                currency_code: "USD".into(),
                value_minor: 250_000,
                ..OpportunityPatch::default()
            },
        },
    )
    .expect("create opportunity")
}

fn quote_ref(external_id: &str) -> HandoffRefInput {
    HandoffRefInput {
        tool: "quoter".into(),
        external_id: external_id.into(),
        label: Some(format!("Q-{external_id}")),
    }
}

/// Move an opportunity into the seeded won stage; returns the updated record.
fn move_to_won(storage: &mut Storage, opportunity: &Opportunity) -> Opportunity {
    let won = list_stages(storage)
        .expect("list stages")
        .into_iter()
        .find(|stage| stage.kind == StageKind::Won)
        .expect("won stage seeded");
    move_opportunity_stage(
        storage,
        MoveOpportunityStageRequest {
            actor: Actor::User,
            opportunity_id: opportunity.id.clone(),
            to_stage_id: won.id,
            lost_reason_id: None,
            expected_version: opportunity.version,
        },
    )
    .expect("move to won")
}

fn command_log_summaries(storage: &Storage, entity_id: &str) -> Vec<String> {
    let mut statement = storage
        .connection()
        .prepare("SELECT summary FROM command_log WHERE entity_id = ?1 ORDER BY created_at, id")
        .expect("prepare command log query");
    statement
        .query_map([entity_id], |row| row.get(0))
        .expect("query command log")
        .collect::<rusqlite::Result<Vec<String>>>()
        .expect("collect summaries")
}

// ---------------------------------------------------------------------------
// Quote link/unlink
// ---------------------------------------------------------------------------

#[test]
fn link_and_unlink_quote_round_trip_bumps_versions_and_logs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Homeowner");
    let opportunity = make_opportunity(&mut storage, "Backyard privacy fence", &contact.id);

    let linked = link_quote(
        &mut storage,
        LinkQuoteRequest {
            actor: Actor::User,
            opportunity_id: opportunity.id.clone(),
            expected_version: opportunity.version,
            quote_ref: quote_ref("123"),
        },
    )
    .expect("link quote");
    let reference = linked.quote_ref.as_ref().expect("quote ref stored");
    assert_eq!(reference.tool, "quoter");
    assert_eq!(reference.external_id, "123");
    assert_eq!(reference.label.as_deref(), Some("Q-123"));
    assert!(!reference.linked_at.is_empty());
    assert_eq!(linked.version, opportunity.version + 1);
    assert!(linked.job_ref.is_none());

    let unlinked = unlink_quote(
        &mut storage,
        UnlinkHandoffRequest {
            actor: Actor::User,
            opportunity_id: opportunity.id.clone(),
            expected_version: linked.version,
        },
    )
    .expect("unlink quote");
    assert!(unlinked.quote_ref.is_none());
    assert_eq!(unlinked.version, linked.version + 1);

    let summaries = command_log_summaries(&storage, &opportunity.id);
    assert!(summaries.iter().any(|s| s.contains("linked quote Q-123")));
    assert!(summaries.iter().any(|s| s.contains("unlinked quote")));

    // The stored ref survives a re-read through the query path.
    let detail = get_opportunity(&storage, &opportunity.id).expect("get opportunity");
    assert!(detail.opportunity.quote_ref.is_none());
}

#[test]
fn stale_version_on_link_quote_is_a_version_conflict() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Homeowner");
    let opportunity = make_opportunity(&mut storage, "Backyard privacy fence", &contact.id);

    let error = link_quote(
        &mut storage,
        LinkQuoteRequest {
            actor: Actor::User,
            opportunity_id: opportunity.id.clone(),
            expected_version: opportunity.version + 5,
            quote_ref: quote_ref("777"),
        },
    )
    .expect_err("stale version must fail");
    assert_eq!(error.kind(), "version_conflict");
    assert!(matches!(error, ApplicationError::VersionConflict { .. }));
}

// ---------------------------------------------------------------------------
// Job link — won-stage rule
// ---------------------------------------------------------------------------

#[test]
fn link_job_is_rejected_unless_the_opportunity_is_won() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Homeowner");
    let opportunity = make_opportunity(&mut storage, "Backyard privacy fence", &contact.id);

    // Still in the first open stage — a job hand-off makes no sense yet.
    let error = link_job(
        &mut storage,
        LinkJobRequest {
            actor: Actor::User,
            opportunity_id: opportunity.id.clone(),
            expected_version: opportunity.version,
            job_ref: HandoffRefInput {
                tool: "contractorproject".into(),
                external_id: "job-9".into(),
                label: None,
            },
        },
    )
    .expect_err("job link on open stage must fail");
    assert_eq!(error.kind(), "validation_failed");
    assert!(error.to_string().contains("won"));

    // Winning the opportunity unlocks the job link.
    let won = move_to_won(&mut storage, &opportunity);
    let linked = link_job(
        &mut storage,
        LinkJobRequest {
            actor: Actor::User,
            opportunity_id: won.id.clone(),
            expected_version: won.version,
            job_ref: HandoffRefInput {
                tool: "contractorproject".into(),
                external_id: "job-9".into(),
                label: Some("Fence install".into()),
            },
        },
    )
    .expect("link job on won opportunity");
    let reference = linked.job_ref.expect("job ref stored");
    assert_eq!(reference.tool, "contractorproject");
    assert_eq!(reference.external_id, "job-9");
    assert_eq!(linked.version, won.version + 1);
}

// ---------------------------------------------------------------------------
// Envelope export
// ---------------------------------------------------------------------------

#[test]
fn envelope_export_writes_schema_v1_json_with_money_and_channels() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Homeowner");
    let opportunity = make_opportunity(&mut storage, "Backyard privacy fence", &contact.id);
    let linked = link_quote(
        &mut storage,
        LinkQuoteRequest {
            actor: Actor::User,
            opportunity_id: opportunity.id.clone(),
            expected_version: opportunity.version,
            quote_ref: quote_ref("123"),
        },
    )
    .expect("link quote");

    let destination = temp.path().join("exports").join("handoff.json");
    let report = export_handoff_envelope(
        &mut storage,
        &linked.id,
        destination.to_str().expect("utf-8 path"),
        false,
    )
    .expect("export envelope");
    assert_eq!(report.schema_version, 1);

    let raw = std::fs::read_to_string(&destination).expect("read envelope file");
    let envelope: serde_json::Value = serde_json::from_str(&raw).expect("valid JSON");

    assert_eq!(envelope["schemaVersion"], 1);
    assert_eq!(envelope["kind"], "opportunity_handoff");
    assert_eq!(envelope["product"]["name"], "ContractorCRM");
    assert!(envelope["exportedAt"].is_string());

    let wire = &envelope["opportunity"];
    assert_eq!(wire["id"], linked.id.as_str());
    assert_eq!(wire["stageName"], "Lead");
    assert_eq!(wire["value"]["valueMinor"], 250_000); // integer minor units
    assert_eq!(wire["value"]["currencyCode"], "USD");
    assert_eq!(wire["quoteRef"]["externalId"], "123");
    assert!(wire["jobRef"].is_null());

    let channels = envelope["contact"]["channels"]
        .as_array()
        .expect("contact channels present");
    assert_eq!(channels.len(), 2);
    assert!(envelope["company"].is_null()); // no company linked

    let summaries = command_log_summaries(&storage, &linked.id);
    assert!(summaries
        .iter()
        .any(|s| s.contains("exported hand-off envelope")));
}

#[test]
fn envelope_export_refuses_an_existing_destination_without_overwrite() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Homeowner");
    let opportunity = make_opportunity(&mut storage, "Backyard privacy fence", &contact.id);
    let destination = temp.path().join("handoff.json");
    let destination_text = destination.to_str().expect("utf-8 path");

    export_handoff_envelope(&mut storage, &opportunity.id, destination_text, false)
        .expect("first export");

    let error = export_handoff_envelope(&mut storage, &opportunity.id, destination_text, false)
        .expect_err("second export without overwrite must fail");
    assert_eq!(error.kind(), "validation_failed");
    assert!(error.to_string().contains("already exists"));

    // Overwrite replaces the file cleanly.
    export_handoff_envelope(&mut storage, &opportunity.id, destination_text, true)
        .expect("export with overwrite");
}

// ---------------------------------------------------------------------------
// Migration 003
// ---------------------------------------------------------------------------

fn opportunity_columns(storage: &Storage) -> Vec<String> {
    let mut statement = storage
        .connection()
        .prepare("SELECT name FROM pragma_table_info('opportunities') ORDER BY name")
        .expect("prepare column listing");
    statement
        .query_map([], |row| row.get(0))
        .expect("query columns")
        .collect::<rusqlite::Result<Vec<String>>>()
        .expect("collect columns")
}

const REF_COLUMNS: &[&str] = &[
    "quote_tool",
    "quote_external_id",
    "quote_label",
    "quote_linked_at",
    "job_tool",
    "job_external_id",
    "job_label",
    "job_linked_at",
];

#[test]
fn migration_003_applies_on_fresh_and_existing_databases() {
    let temp = tempfile::tempdir().expect("tempdir");
    let database_path = temp.path().join("contractorcrm.sqlite3");

    // Fresh database: all three migrations apply and the ref columns exist.
    let storage = Storage::open(&database_path).expect("fresh open");
    let columns = opportunity_columns(&storage);
    for column in REF_COLUMNS {
        assert!(columns.contains(&column.to_string()), "missing {column}");
    }

    // Simulate a pre-v3 database: drop the ref columns and the ledger row,
    // then reopen — migration 003 must re-apply forward.
    for column in REF_COLUMNS {
        storage
            .connection()
            .execute_batch(&format!("ALTER TABLE opportunities DROP COLUMN {column};"))
            .expect("drop ref column");
    }
    storage
        .connection()
        .execute("DELETE FROM schema_migrations WHERE version = 3", [])
        .expect("forget migration 003");
    drop(storage);

    let reopened = Storage::open(&database_path).expect("reopen existing");
    let columns = opportunity_columns(&reopened);
    for column in REF_COLUMNS {
        assert!(columns.contains(&column.to_string()), "missing {column}");
    }
    let applied: i64 = reopened
        .connection()
        .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .expect("read applied version");
    assert_eq!(applied, latest_migration_version());
}
