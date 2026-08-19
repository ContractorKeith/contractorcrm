//! End-to-end hand-off: a won opportunity leaves ContractorCRM as a versioned
//! envelope, ContractorProject's `handoff-import` binary turns it into a job,
//! and the resulting job id is linked back onto the opportunity.
//!
//! Ignored by default and skipped unless `HANDOFF_IMPORT_BIN` points at a built
//! sibling binary — CI never needs ContractorProject checked out. Run it with
//! `scripts/handoff_e2e.sh`.

use contractorcrm_lib::application::{
    create_contact, create_opportunity, export_handoff_envelope, get_opportunity, link_job,
    list_stages, move_opportunity_stage, ChannelInput, ContactPatch, CreateContactRequest,
    CreateOpportunityRequest, HandoffRefInput, LinkJobRequest, MoveOpportunityStageRequest,
    OpportunityPatch,
};
use contractorcrm_lib::domain::{Actor, StageKind};
use contractorcrm_lib::storage::Storage;
use std::path::Path;
use std::process::Command;

/// The sibling binary's success line: `{"jobId","jobName","createdAt"}`.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImportedJob {
    job_id: String,
    job_name: String,
    created_at: String,
}

/// Run `handoff-import` against a fresh sibling database and parse its line.
fn run_import(binary: &Path, envelope: &Path, database: &Path) -> ImportedJob {
    let output = Command::new(binary)
        .arg("--envelope")
        .arg(envelope)
        .arg("--database")
        .arg(database)
        .arg("--timezone")
        .arg("America/New_York")
        .output()
        .expect("spawn handoff-import");
    let stdout = String::from_utf8(output.stdout).expect("utf-8 stdout");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "handoff-import failed ({:?}): {stderr}",
        output.status.code()
    );
    let line = stdout.trim();
    assert!(
        !line.contains('\n'),
        "expected exactly one JSON line: {line}"
    );
    serde_json::from_str(line).unwrap_or_else(|error| panic!("parse {line:?}: {error}"))
}

#[test]
#[ignore = "requires HANDOFF_IMPORT_BIN — run scripts/handoff_e2e.sh"]
fn won_opportunity_becomes_a_contractorproject_job_and_links_back() {
    let Ok(binary) = std::env::var("HANDOFF_IMPORT_BIN") else {
        println!(
            "skipping: set HANDOFF_IMPORT_BIN to ContractorProject's handoff-import binary \
             (scripts/handoff_e2e.sh builds it)"
        );
        return;
    };
    let binary = Path::new(&binary);
    assert!(
        binary.is_file(),
        "HANDOFF_IMPORT_BIN is not a file: {binary:?}"
    );

    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = Storage::open_in_app_data(temp.path()).expect("open storage");

    // A real CRM record set: contact, opportunity, moved into the won stage.
    let contact = create_contact(
        &mut storage,
        CreateContactRequest {
            actor: Actor::User,
            contact: ContactPatch {
                display_name: Some("Dana Homeowner".into()),
                kind: "lead".into(),
                channels: vec![ChannelInput {
                    kind: "phone".into(),
                    label: Some("mobile".into()),
                    value: "555-0100".into(),
                    preferred: true,
                }],
                ..ContactPatch::default()
            },
        },
    )
    .expect("create contact");

    let opportunity = create_opportunity(
        &mut storage,
        CreateOpportunityRequest {
            actor: Actor::User,
            stage_id: None,
            opportunity: OpportunityPatch {
                name: "Backyard privacy fence".into(),
                contact_id: Some(contact.id.clone()),
                currency_code: "USD".into(),
                value_minor: 250_000,
                ..OpportunityPatch::default()
            },
        },
    )
    .expect("create opportunity");

    let won_stage = list_stages(&storage)
        .expect("list stages")
        .into_iter()
        .find(|stage| stage.kind == StageKind::Won)
        .expect("won stage seeded");
    let won = move_opportunity_stage(
        &mut storage,
        MoveOpportunityStageRequest {
            actor: Actor::User,
            opportunity_id: opportunity.id.clone(),
            to_stage_id: won_stage.id,
            lost_reason_id: None,
            expected_version: opportunity.version,
        },
    )
    .expect("move to won");

    // Export the versioned envelope — the only interface between the modules.
    let envelope_path = temp.path().join("exports").join("handoff.json");
    let report = export_handoff_envelope(
        &mut storage,
        &won.id,
        envelope_path.to_str().expect("utf-8 path"),
        false,
    )
    .expect("export envelope");
    assert_eq!(report.schema_version, 1);

    // Hand it to ContractorProject against a fresh sibling database.
    let sibling_database = temp.path().join("contractorproject.sqlite3");
    let imported = run_import(binary, &envelope_path, &sibling_database);
    assert_eq!(imported.job_name, "Backyard privacy fence");
    assert!(!imported.job_id.is_empty());
    assert!(!imported.created_at.is_empty());

    // Link the job back onto the won opportunity with the version check.
    let linked = link_job(
        &mut storage,
        LinkJobRequest {
            actor: Actor::User,
            opportunity_id: won.id.clone(),
            expected_version: won.version,
            job_ref: HandoffRefInput {
                tool: "contractorproject".into(),
                external_id: imported.job_id.clone(),
                label: Some(imported.job_name.clone()),
            },
        },
    )
    .expect("link job");
    assert_eq!(linked.version, won.version + 1);

    // Re-read through the query path: the stored ref must match the job.
    let detail = get_opportunity(&storage, &won.id).expect("get opportunity");
    let stored = detail.opportunity.job_ref.expect("job ref stored");
    assert_eq!(stored.tool, "contractorproject");
    assert_eq!(stored.external_id, imported.job_id);
    assert_eq!(stored.label.as_deref(), Some(imported.job_name.as_str()));
    assert!(!stored.linked_at.is_empty());

    // A second import of the same envelope is a second job — no dedup magic.
    let second = run_import(binary, &envelope_path, &sibling_database);
    assert_ne!(second.job_id, imported.job_id);

    println!(
        "hand-off ok: opportunity {} -> job {} ({})",
        won.id, imported.job_id, imported.job_name
    );
}
