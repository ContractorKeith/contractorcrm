//! Integration tests for the pipeline domain — seeded defaults, opportunity
//! CRUD, stage moves with append-only history, lost-reason rules, version
//! conflicts, and integer-only money on the wire.

use contractorcrm_lib::application::{
    archive_opportunity, create_contact, create_opportunity, get_opportunity, list_lost_reasons,
    list_opportunities, list_stages, move_opportunity_stage, unarchive_opportunity,
    update_opportunity, update_stage, ArchiveRequest, ContactPatch, CreateContactRequest,
    CreateOpportunityRequest, MoveOpportunityStageRequest, OpportunityPatch,
    UpdateOpportunityRequest, UpdateStageRequest,
};
use contractorcrm_lib::domain::{Actor, Contact, Stage, StageKind};
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

fn opportunity_patch(name: &str, contact_id: Option<&str>) -> OpportunityPatch {
    OpportunityPatch {
        name: name.into(),
        contact_id: contact_id.map(Into::into),
        currency_code: "usd".into(), // normalized to USD by validation
        value_minor: 250_000,
        ..OpportunityPatch::default()
    }
}

fn make_opportunity(
    storage: &mut Storage,
    name: &str,
    contact_id: &str,
) -> contractorcrm_lib::domain::Opportunity {
    create_opportunity(
        storage,
        CreateOpportunityRequest {
            actor: Actor::User,
            stage_id: None,
            opportunity: opportunity_patch(name, Some(contact_id)),
        },
    )
    .expect("create opportunity")
}

/// Find one seeded stage by kind (open stages by name instead).
fn stage_by_name(stages: &[Stage], name: &str) -> Stage {
    stages
        .iter()
        .find(|stage| stage.name == name)
        .unwrap_or_else(|| panic!("stage {name} not found"))
        .clone()
}

// ---------------------------------------------------------------------------
// Seeding and migrations
// ---------------------------------------------------------------------------

#[test]
fn fresh_database_seeds_default_pipeline_stages_and_lost_reasons() {
    let temp = tempfile::tempdir().expect("tempdir");
    let storage = open_storage(&temp);

    let stages = list_stages(&storage).expect("list stages");
    let names: Vec<&str> = stages.iter().map(|stage| stage.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "Lead",
            "Estimating",
            "Proposal Sent",
            "Negotiation",
            "Won",
            "Lost"
        ]
    );
    let kinds: Vec<StageKind> = stages.iter().map(|stage| stage.kind).collect();
    assert_eq!(
        kinds,
        vec![
            StageKind::Open,
            StageKind::Open,
            StageKind::Open,
            StageKind::Open,
            StageKind::Won,
            StageKind::Lost
        ]
    );
    assert!(stages.iter().all(|stage| stage.version == 1));

    let reasons = list_lost_reasons(&storage).expect("list lost reasons");
    let labels: Vec<&str> = reasons.iter().map(|reason| reason.label.as_str()).collect();
    assert_eq!(
        labels,
        vec![
            "Price",
            "Timing",
            "Went with competitor",
            "No response",
            "Out of scope"
        ]
    );
    assert!(reasons.iter().all(|reason| reason.active));
}

#[test]
fn reopening_the_database_does_not_reapply_migration_002() {
    let temp = tempfile::tempdir().expect("tempdir");
    drop(open_storage(&temp)); // first open applies and seeds
    let storage = open_storage(&temp); // second open must be a no-op

    assert_eq!(list_stages(&storage).expect("list stages").len(), 6);
    assert_eq!(
        list_lost_reasons(&storage)
            .expect("list lost reasons")
            .len(),
        5
    );
}

// ---------------------------------------------------------------------------
// Opportunity CRUD
// ---------------------------------------------------------------------------

#[test]
fn create_update_and_list_opportunity_happy_path() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Homeowner");

    let opportunity = make_opportunity(&mut storage, "Backyard privacy fence", &contact.id);
    let stages = list_stages(&storage).expect("list stages");
    let lead = stage_by_name(&stages, "Lead");
    assert_eq!(opportunity.stage_id, lead.id); // first open stage by default
    assert_eq!(opportunity.value.value_minor, 250_000);
    assert_eq!(opportunity.value.currency_code, "USD");
    assert_eq!(opportunity.version, 1);

    let updated = update_opportunity(
        &mut storage,
        UpdateOpportunityRequest {
            actor: Actor::Agent,
            opportunity_id: opportunity.id.clone(),
            expected_version: 1,
            patch: OpportunityPatch {
                probability_percent: Some(60),
                notes: Some("gate added to scope".into()),
                ..opportunity_patch("Backyard fence + gate", Some(&contact.id))
            },
        },
    )
    .expect("update opportunity");
    assert_eq!(updated.name, "Backyard fence + gate");
    assert_eq!(updated.probability_percent, Some(60));
    assert_eq!(updated.version, 2);
    assert_eq!(updated.stage_id, lead.id); // updates never move stages

    let listed = list_opportunities(&storage, false).expect("list opportunities");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].stage_name, "Lead");
    assert_eq!(
        listed[0].contact_display_name.as_deref(),
        Some("Dana Homeowner")
    );
    assert_eq!(listed[0].company_name, None);

    let detail = get_opportunity(&storage, &opportunity.id).expect("get opportunity");
    assert_eq!(detail.opportunity, updated);
    // Creation wrote the initial history row (from nothing into Lead).
    assert_eq!(detail.stage_history.len(), 1);
    assert_eq!(detail.stage_history[0].from_stage_id, None);
    assert_eq!(detail.stage_history[0].to_stage_id, lead.id);
}

#[test]
fn create_opportunity_without_contact_and_company_is_rejected() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);

    let rejected = create_opportunity(
        &mut storage,
        CreateOpportunityRequest {
            actor: Actor::User,
            stage_id: None,
            opportunity: opportunity_patch("Orphan opportunity", None),
        },
    )
    .expect_err("no contact or company must fail");
    assert_eq!(rejected.kind(), "validation_failed");
    assert!(
        matches!(
            &rejected,
            ApplicationError::ValidationFailed { code, .. }
                if *code == "opportunity_needs_contact_or_company"
        ),
        "unexpected error: {rejected:?}"
    );
    assert!(list_opportunities(&storage, true).expect("list").is_empty());
}

#[test]
fn create_opportunity_validation_failures() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Homeowner");

    // Currency code must be three letters.
    let bad_currency = create_opportunity(
        &mut storage,
        CreateOpportunityRequest {
            actor: Actor::User,
            stage_id: None,
            opportunity: OpportunityPatch {
                currency_code: "$".into(),
                ..opportunity_patch("Fence", Some(&contact.id))
            },
        },
    )
    .expect_err("bad currency must fail");
    assert!(
        matches!(
            &bad_currency,
            ApplicationError::InvalidInput { field, .. } if field == "currencyCode"
        ),
        "unexpected error: {bad_currency:?}"
    );

    // Probability outside 0–100.
    let bad_probability = create_opportunity(
        &mut storage,
        CreateOpportunityRequest {
            actor: Actor::User,
            stage_id: None,
            opportunity: OpportunityPatch {
                probability_percent: Some(120),
                ..opportunity_patch("Fence", Some(&contact.id))
            },
        },
    )
    .expect_err("probability over 100 must fail");
    assert!(
        matches!(
            &bad_probability,
            ApplicationError::InvalidInput { field, .. } if field == "probabilityPercent"
        ),
        "unexpected error: {bad_probability:?}"
    );

    // Negative money.
    let negative_value = create_opportunity(
        &mut storage,
        CreateOpportunityRequest {
            actor: Actor::User,
            stage_id: None,
            opportunity: OpportunityPatch {
                value_minor: -1,
                ..opportunity_patch("Fence", Some(&contact.id))
            },
        },
    )
    .expect_err("negative value must fail");
    assert!(
        matches!(
            &negative_value,
            ApplicationError::InvalidInput { field, .. } if field == "valueMinor"
        ),
        "unexpected error: {negative_value:?}"
    );

    // Unknown contact id → not_found, not invalid_input.
    let missing_contact = create_opportunity(
        &mut storage,
        CreateOpportunityRequest {
            actor: Actor::User,
            stage_id: None,
            opportunity: opportunity_patch("Fence", Some("no-such-contact")),
        },
    )
    .expect_err("unknown contact must fail");
    assert_eq!(missing_contact.kind(), "not_found");

    assert!(list_opportunities(&storage, true).expect("list").is_empty());
}

// ---------------------------------------------------------------------------
// Stage moves and history
// ---------------------------------------------------------------------------

#[test]
fn stage_move_appends_history_with_actor_and_from_to() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Homeowner");
    let opportunity = make_opportunity(&mut storage, "Fence", &contact.id);
    let stages = list_stages(&storage).expect("list stages");
    let lead = stage_by_name(&stages, "Lead");
    let estimating = stage_by_name(&stages, "Estimating");

    let moved = move_opportunity_stage(
        &mut storage,
        MoveOpportunityStageRequest {
            actor: Actor::Agent,
            opportunity_id: opportunity.id.clone(),
            to_stage_id: estimating.id.clone(),
            lost_reason_id: None,
            expected_version: 1,
        },
    )
    .expect("move to estimating");
    assert_eq!(moved.stage_id, estimating.id);
    assert_eq!(moved.version, 2);

    let detail = get_opportunity(&storage, &opportunity.id).expect("get opportunity");
    assert_eq!(detail.stage_history.len(), 2); // create + move
    let entry = &detail.stage_history[1];
    assert_eq!(entry.from_stage_id.as_deref(), Some(lead.id.as_str()));
    assert_eq!(entry.to_stage_id, estimating.id);
    assert_eq!(entry.actor, Actor::Agent);
    assert_eq!(entry.lost_reason_id, None);
}

#[test]
fn lost_move_without_reason_fails_with_missing_lost_reason() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Homeowner");
    let opportunity = make_opportunity(&mut storage, "Fence", &contact.id);
    let stages = list_stages(&storage).expect("list stages");
    let lost = stage_by_name(&stages, "Lost");

    let rejected = move_opportunity_stage(
        &mut storage,
        MoveOpportunityStageRequest {
            actor: Actor::User,
            opportunity_id: opportunity.id.clone(),
            to_stage_id: lost.id.clone(),
            lost_reason_id: None,
            expected_version: 1,
        },
    )
    .expect_err("lost without reason must fail");
    assert_eq!(rejected.kind(), "missing_lost_reason");

    // Nothing moved and no history row leaked from the failed attempt.
    let detail = get_opportunity(&storage, &opportunity.id).expect("get opportunity");
    assert_eq!(detail.opportunity.version, 1);
    assert_eq!(detail.stage_history.len(), 1);
}

#[test]
fn lost_move_with_reason_stores_it_and_moving_away_clears_it() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Homeowner");
    let opportunity = make_opportunity(&mut storage, "Fence", &contact.id);
    let stages = list_stages(&storage).expect("list stages");
    let lost = stage_by_name(&stages, "Lost");
    let estimating = stage_by_name(&stages, "Estimating");
    let price = &list_lost_reasons(&storage).expect("list lost reasons")[0];

    let lost_opportunity = move_opportunity_stage(
        &mut storage,
        MoveOpportunityStageRequest {
            actor: Actor::User,
            opportunity_id: opportunity.id.clone(),
            to_stage_id: lost.id.clone(),
            lost_reason_id: Some(price.id.clone()),
            expected_version: 1,
        },
    )
    .expect("move to lost with reason");
    assert_eq!(lost_opportunity.stage_id, lost.id);
    assert_eq!(
        lost_opportunity.lost_reason_id.as_deref(),
        Some(price.id.as_str())
    );

    // The history row carries the reason too.
    let detail = get_opportunity(&storage, &opportunity.id).expect("get opportunity");
    assert_eq!(
        detail.stage_history[1].lost_reason_id.as_deref(),
        Some(price.id.as_str())
    );

    // Reopening the deal clears the stored lost reason.
    let reopened = move_opportunity_stage(
        &mut storage,
        MoveOpportunityStageRequest {
            actor: Actor::User,
            opportunity_id: opportunity.id.clone(),
            to_stage_id: estimating.id.clone(),
            lost_reason_id: None,
            expected_version: 2,
        },
    )
    .expect("move away from lost");
    assert_eq!(reopened.stage_id, estimating.id);
    assert_eq!(reopened.lost_reason_id, None);
}

#[test]
fn stale_move_is_rejected_with_version_conflict() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Homeowner");
    let opportunity = make_opportunity(&mut storage, "Fence", &contact.id);
    let stages = list_stages(&storage).expect("list stages");
    let estimating = stage_by_name(&stages, "Estimating");
    let negotiation = stage_by_name(&stages, "Negotiation");

    move_opportunity_stage(
        &mut storage,
        MoveOpportunityStageRequest {
            actor: Actor::User,
            opportunity_id: opportunity.id.clone(),
            to_stage_id: estimating.id,
            lost_reason_id: None,
            expected_version: 1,
        },
    )
    .expect("first move");

    let stale = move_opportunity_stage(
        &mut storage,
        MoveOpportunityStageRequest {
            actor: Actor::User,
            opportunity_id: opportunity.id.clone(),
            to_stage_id: negotiation.id,
            lost_reason_id: None,
            expected_version: 1,
        },
    )
    .expect_err("stale move must fail");
    assert_eq!(stale.kind(), "version_conflict");
    assert!(
        matches!(
            &stale,
            ApplicationError::VersionConflict {
                expected: 1,
                current: 2,
                ..
            }
        ),
        "unexpected error: {stale:?}"
    );
}

// ---------------------------------------------------------------------------
// Archive, stage renames, and the wire shape
// ---------------------------------------------------------------------------

#[test]
fn opportunity_archive_unarchive_round_trip_and_list_filtering() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Homeowner");
    let opportunity = make_opportunity(&mut storage, "Fence", &contact.id);

    let archived = archive_opportunity(
        &mut storage,
        ArchiveRequest {
            actor: Actor::User,
            id: opportunity.id.clone(),
            expected_version: 1,
        },
    )
    .expect("archive opportunity");
    assert!(archived.archived_at.is_some());
    assert_eq!(archived.version, 2);
    assert!(list_opportunities(&storage, false)
        .expect("list active")
        .is_empty());
    assert_eq!(
        list_opportunities(&storage, true).expect("list all").len(),
        1
    );

    let unarchived = unarchive_opportunity(
        &mut storage,
        ArchiveRequest {
            actor: Actor::User,
            id: opportunity.id.clone(),
            expected_version: 2,
        },
    )
    .expect("unarchive opportunity");
    assert!(unarchived.archived_at.is_none());
    assert_eq!(unarchived.version, 3);
    assert_eq!(
        list_opportunities(&storage, false)
            .expect("list active")
            .len(),
        1
    );
}

#[test]
fn renaming_and_reordering_a_stage_leaves_history_intact() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Homeowner");
    let opportunity = make_opportunity(&mut storage, "Fence", &contact.id);
    let stages = list_stages(&storage).expect("list stages");
    let estimating = stage_by_name(&stages, "Estimating");

    move_opportunity_stage(
        &mut storage,
        MoveOpportunityStageRequest {
            actor: Actor::User,
            opportunity_id: opportunity.id.clone(),
            to_stage_id: estimating.id.clone(),
            lost_reason_id: None,
            expected_version: 1,
        },
    )
    .expect("move to estimating");
    let before = get_opportunity(&storage, &opportunity.id)
        .expect("get opportunity")
        .stage_history;

    // Rename and reorder the stage; history stores ids only, so nothing moves.
    let renamed = update_stage(
        &mut storage,
        UpdateStageRequest {
            actor: Actor::User,
            stage_id: estimating.id.clone(),
            expected_version: 1,
            name: "Measuring & Estimating".into(),
            sort_key: 9,
        },
    )
    .expect("rename stage");
    assert_eq!(renamed.name, "Measuring & Estimating");
    assert_eq!(renamed.sort_key, 9);
    assert_eq!(renamed.version, 2);

    let after = get_opportunity(&storage, &opportunity.id)
        .expect("get opportunity")
        .stage_history;
    assert_eq!(before, after);
    assert_eq!(after[1].to_stage_id, estimating.id);
}

#[test]
fn money_stays_integer_on_the_wire() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Homeowner");
    let opportunity = make_opportunity(&mut storage, "Fence", &contact.id);

    let wire = serde_json::to_value(&opportunity).expect("serialize opportunity");
    let value_minor = &wire["value"]["valueMinor"];
    assert!(value_minor.is_i64(), "money must be an integer: {wire}");
    assert_eq!(value_minor.as_i64(), Some(250_000));
    assert_eq!(wire["value"]["currencyCode"], "USD");
}
