//! Integration tests for the needs-attention seam — seeded scenarios through
//! the application query, threshold get/set persistence, and the read-time
//! last-contacted / next-task projections on the list views.

use chrono::{Duration, SecondsFormat, Utc};
use contractorcrm_lib::application::{
    complete_task, create_contact, create_opportunity, create_task, get_attention_flags,
    get_attention_thresholds, list_contacts, list_opportunities, log_activity,
    move_opportunity_stage, set_attention_thresholds, ActivityPatch, ContactPatch,
    CreateContactRequest, CreateOpportunityRequest, CreateTaskRequest, LogActivityRequest,
    MoveOpportunityStageRequest, OpportunityPatch, SetAttentionThresholdsRequest, TaskPatch,
};
use contractorcrm_lib::attention::AttentionRule;
use contractorcrm_lib::domain::{Actor, Contact, Opportunity, Task};
use contractorcrm_lib::storage::Storage;

fn open_storage(temp: &tempfile::TempDir) -> Storage {
    Storage::open_in_app_data(temp.path()).expect("open storage")
}

/// UTC ISO-8601 timestamp this many days away from now (negative = past).
fn days_from_now(days: i64) -> String {
    (Utc::now() + Duration::days(days)).to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn make_contact(storage: &mut Storage, display_name: &str, kind: &str) -> Contact {
    create_contact(
        storage,
        CreateContactRequest {
            actor: Actor::User,
            contact: ContactPatch {
                display_name: Some(display_name.into()),
                kind: kind.into(),
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
                ..OpportunityPatch::default()
            },
        },
    )
    .expect("create opportunity")
}

/// Move an opportunity into the seeded "Proposal Sent" stage.
fn send_proposal(storage: &mut Storage, opportunity: &Opportunity) -> Opportunity {
    move_opportunity_stage(
        storage,
        MoveOpportunityStageRequest {
            actor: Actor::User,
            opportunity_id: opportunity.id.clone(),
            to_stage_id: "stage-proposal-sent".into(),
            lost_reason_id: None,
            expected_version: opportunity.version,
        },
    )
    .expect("move to proposal sent")
}

fn log_touch(
    storage: &mut Storage,
    parent_type: &str,
    parent_id: &str,
    direction: &str,
    occurred_at: &str,
) {
    log_activity(
        storage,
        LogActivityRequest {
            actor: Actor::User,
            parent_type: parent_type.into(),
            parent_id: parent_id.into(),
            activity: ActivityPatch {
                kind: "call".into(),
                direction: Some(direction.into()),
                occurred_at: Some(occurred_at.into()),
                summary: "Phone call".into(),
                ..ActivityPatch::default()
            },
        },
    )
    .expect("log activity");
}

fn make_task(storage: &mut Storage, title: &str, patch: TaskPatch) -> Task {
    create_task(
        storage,
        CreateTaskRequest {
            actor: Actor::User,
            task: TaskPatch {
                title: title.into(),
                ..patch
            },
        },
    )
    .expect("create task")
}

// ---------------------------------------------------------------------------
// Flags through the application query
// ---------------------------------------------------------------------------

#[test]
fn seeded_scenario_yields_the_expected_flags_in_severity_order() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    // Records are created "now"; the reference time sits 10 days ahead so the
    // proposal threshold (7 days) is crossed without backdating stage moves.
    let reference_time = days_from_now(10);

    // Stale lead: last activity 40 days before the reference time.
    let stale_lead = make_contact(&mut storage, "Stale Sam", "lead");
    log_touch(
        &mut storage,
        "contact",
        &stale_lead.id,
        "outbound",
        &days_from_now(-30),
    );
    // Fresh lead: touched now, only 10 days before the reference — no flag.
    let fresh_lead = make_contact(&mut storage, "Fresh Fran", "lead");
    log_touch(
        &mut storage,
        "contact",
        &fresh_lead.id,
        "outbound",
        &days_from_now(0),
    );

    // Proposal with no inbound response since entering the stage — flags.
    let client = make_contact(&mut storage, "Carla Client", "client");
    let pending = make_opportunity(&mut storage, "Pending Fence", &client.id);
    let pending = send_proposal(&mut storage, &pending);
    // Proposal answered by an inbound call after entering the stage — no flag.
    let answered = make_opportunity(&mut storage, "Answered Fence", &client.id);
    let answered = send_proposal(&mut storage, &answered);
    log_touch(
        &mut storage,
        "opportunity",
        &answered.id,
        "inbound",
        &days_from_now(1),
    );

    // Overdue open task flags; future-due and completed ones do not.
    let overdue = make_task(
        &mut storage,
        "Call the county office",
        TaskPatch {
            due_at: Some(days_from_now(-2)),
            ..TaskPatch::default()
        },
    );
    make_task(
        &mut storage,
        "Order pickets",
        TaskPatch {
            due_at: Some(days_from_now(30)),
            ..TaskPatch::default()
        },
    );
    let done = make_task(
        &mut storage,
        "Send invoice",
        TaskPatch {
            due_at: Some(days_from_now(-5)),
            ..TaskPatch::default()
        },
    );
    complete_task(
        &mut storage,
        contractorcrm_lib::application::CompleteTaskRequest {
            actor: Actor::User,
            task_id: done.id.clone(),
            expected_version: done.version,
            log_activity: false,
        },
    )
    .expect("complete task");

    let flags =
        get_attention_flags(&storage, Some(reference_time.clone())).expect("attention flags");
    assert_eq!(flags.len(), 3, "flags: {flags:?}");

    // Severity order: overdue task, then proposal, then stale lead.
    assert_eq!(flags[0].rule, AttentionRule::OverdueTask);
    assert_eq!(flags[0].id, format!("overdue_task:{}", overdue.id));
    // Due ~12 days before the reference; clock skew between helper calls can
    // land on 11 — the pure unit tests pin the exact day math.
    assert!(flags[0].explanation.contains("day(s) overdue"));

    assert_eq!(flags[1].rule, AttentionRule::ProposalNoResponse);
    assert_eq!(flags[1].id, format!("proposal_no_response:{}", pending.id));
    assert!(flags[1].explanation.contains("Pending Fence"));
    assert!(flags[1].explanation.contains("no inbound response"));

    assert_eq!(flags[2].rule, AttentionRule::StaleLead);
    assert_eq!(flags[2].id, format!("stale_lead:{}", stale_lead.id));
    assert!(flags[2].explanation.contains("Stale Sam"));

    // Same inputs, same flags — deterministic ids and ordering.
    let again = get_attention_flags(&storage, Some(reference_time)).expect("attention flags");
    assert_eq!(flags, again);
}

#[test]
fn related_opportunity_activity_keeps_a_lead_from_going_stale() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let reference_time = days_from_now(10);

    // The lead itself is silent, but a linked opportunity was touched now.
    let lead = make_contact(&mut storage, "Lena Lead", "lead");
    let opportunity = make_opportunity(&mut storage, "Lena's Gate", &lead.id);
    log_touch(
        &mut storage,
        "opportunity",
        &opportunity.id,
        "outbound",
        &days_from_now(0),
    );
    // A silent lead created 10 days before the reference is under 21 — no flag
    // either; its clock runs from created_at.
    make_contact(&mut storage, "New Ned", "lead");

    let flags = get_attention_flags(&storage, Some(reference_time)).expect("attention flags");
    assert!(
        !flags
            .iter()
            .any(|flag| flag.rule == AttentionRule::StaleLead),
        "flags: {flags:?}"
    );
}

#[test]
fn invalid_reference_time_is_rejected() {
    let temp = tempfile::tempdir().expect("tempdir");
    let storage = open_storage(&temp);

    let error = get_attention_flags(&storage, Some("not a timestamp".into()))
        .expect_err("invalid reference time must fail");
    assert_eq!(error.kind(), "invalid_input");
}

// ---------------------------------------------------------------------------
// Thresholds — defaults, persistence, validation
// ---------------------------------------------------------------------------

#[test]
fn thresholds_default_then_persist_and_change_the_flags() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let reference_time = days_from_now(10);

    let defaults = get_attention_thresholds(&storage).expect("default thresholds");
    assert_eq!(defaults.stale_lead_days, 21);
    assert_eq!(defaults.proposal_no_response_days, 7);
    assert_eq!(defaults.proposal_stage_name, "Proposal Sent");

    // Stale at the defaults: silent for 40 days by the reference time.
    let lead = make_contact(&mut storage, "Stale Sam", "lead");
    log_touch(
        &mut storage,
        "contact",
        &lead.id,
        "outbound",
        &days_from_now(-30),
    );
    // In Proposal Sent for 10 days by the reference time.
    let client = make_contact(&mut storage, "Carla Client", "client");
    let opportunity = make_opportunity(&mut storage, "Pending Fence", &client.id);
    send_proposal(&mut storage, &opportunity);

    let flags =
        get_attention_flags(&storage, Some(reference_time.clone())).expect("attention flags");
    assert_eq!(flags.len(), 2);

    // Loosen both thresholds past the seeded ages — both flags disappear.
    let updated = set_attention_thresholds(
        &mut storage,
        SetAttentionThresholdsRequest {
            actor: Actor::User,
            stale_lead_days: 50,
            proposal_no_response_days: 30,
            proposal_stage_name: None,
        },
    )
    .expect("set thresholds");
    assert_eq!(updated.stale_lead_days, 50);
    assert_eq!(updated.proposal_no_response_days, 30);
    assert_eq!(updated.proposal_stage_name, "Proposal Sent");
    assert_eq!(
        get_attention_thresholds(&storage).expect("reread thresholds"),
        updated
    );

    let flags = get_attention_flags(&storage, Some(reference_time)).expect("attention flags");
    assert!(flags.is_empty(), "flags: {flags:?}");
}

#[test]
fn threshold_day_counts_must_be_positive() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);

    for (stale, proposal) in [(0, 7), (21, -1)] {
        let error = set_attention_thresholds(
            &mut storage,
            SetAttentionThresholdsRequest {
                actor: Actor::User,
                stale_lead_days: stale,
                proposal_no_response_days: proposal,
                proposal_stage_name: None,
            },
        )
        .expect_err("non-positive day counts must fail");
        assert_eq!(error.kind(), "invalid_input");
    }
}

// ---------------------------------------------------------------------------
// List projections — last_contacted_at and next_open_task_due_at
// ---------------------------------------------------------------------------

#[test]
fn contact_list_carries_last_contacted_and_next_task_due() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);

    let contact = make_contact(&mut storage, "Dana Homeowner", "client");
    // Older touch on the contact, newer touch on a linked opportunity — the
    // projection takes the latest across both.
    log_touch(
        &mut storage,
        "contact",
        &contact.id,
        "outbound",
        "2026-08-01T12:00:00.000Z",
    );
    let opportunity = make_opportunity(&mut storage, "Dana's Fence", &contact.id);
    log_touch(
        &mut storage,
        "opportunity",
        &opportunity.id,
        "inbound",
        "2026-08-05T12:00:00.000Z",
    );
    // Two open contact tasks — the earlier due date wins.
    for due_at in ["2026-08-20T12:00:00.000Z", "2026-08-18T12:00:00.000Z"] {
        make_task(
            &mut storage,
            "Follow up",
            TaskPatch {
                parent_type: Some("contact".into()),
                parent_id: Some(contact.id.clone()),
                due_at: Some(due_at.into()),
                ..TaskPatch::default()
            },
        );
    }
    // An untouched contact projects nothing.
    make_contact(&mut storage, "Quiet Quinn", "client");

    let items = list_contacts(&storage, false).expect("list contacts");
    let dana = items
        .iter()
        .find(|item| item.contact.id == contact.id)
        .expect("Dana in list");
    assert_eq!(
        dana.last_contacted_at.as_deref(),
        Some("2026-08-05T12:00:00.000Z")
    );
    assert_eq!(
        dana.next_open_task_due_at.as_deref(),
        Some("2026-08-18T12:00:00.000Z")
    );
    let quinn = items
        .iter()
        .find(|item| item.contact.display_name == "Quiet Quinn")
        .expect("Quinn in list");
    assert_eq!(quinn.last_contacted_at, None);
    assert_eq!(quinn.next_open_task_due_at, None);
}

#[test]
fn opportunity_list_carries_last_contacted_and_next_task_due() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);

    let contact = make_contact(&mut storage, "Dana Homeowner", "client");
    let opportunity = make_opportunity(&mut storage, "Dana's Fence", &contact.id);
    log_touch(
        &mut storage,
        "opportunity",
        &opportunity.id,
        "outbound",
        "2026-08-03T12:00:00.000Z",
    );
    make_task(
        &mut storage,
        "Walk the site",
        TaskPatch {
            parent_type: Some("opportunity".into()),
            parent_id: Some(opportunity.id.clone()),
            due_at: Some("2026-08-21T12:00:00.000Z".into()),
            ..TaskPatch::default()
        },
    );
    let bare = make_opportunity(&mut storage, "Bare Opportunity", &contact.id);

    let items = list_opportunities(&storage, false).expect("list opportunities");
    let fence = items
        .iter()
        .find(|item| item.opportunity.id == opportunity.id)
        .expect("fence in list");
    assert_eq!(
        fence.last_contacted_at.as_deref(),
        Some("2026-08-03T12:00:00.000Z")
    );
    assert_eq!(
        fence.next_open_task_due_at.as_deref(),
        Some("2026-08-21T12:00:00.000Z")
    );
    let bare_item = items
        .iter()
        .find(|item| item.opportunity.id == bare.id)
        .expect("bare in list");
    assert_eq!(bare_item.last_contacted_at, None);
    assert_eq!(bare_item.next_open_task_due_at, None);
}
