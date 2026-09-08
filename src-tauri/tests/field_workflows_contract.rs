//! Serialization boundary for the compact field-workflow API additions.

use contractorcrm_lib::{
    application::{self, CaptureLeadRequest},
    domain::Actor,
    record_brief,
    storage::Storage,
};
use serde_json::Value;

const LOCAL_API_SCHEMA: &str = include_str!("../../schemas/v1/local-api.json");

fn schema() -> Value {
    serde_json::from_str(LOCAL_API_SCHEMA).expect("valid local API schema")
}

#[test]
fn field_workflow_wire_types_are_versioned_and_match_camel_case_serialization() {
    let schema = schema();
    let wire_types = schema["wireTypes"].as_object().expect("wire types object");
    for name in [
        "CaptureLeadRequest",
        "CapturedLead",
        "WorkQueue",
        "WorkQueueTask",
        "WorkQueueItem",
        "WorkQueueRecord",
        "RecordBrief",
        "RecordBriefRecord",
        "LinkedRecords",
    ] {
        assert!(wire_types.contains_key(name), "{name} is versioned");
    }
    assert_eq!(wire_types["WorkQueueItem"]["tag"], "kind");
    assert_eq!(wire_types["RecordBriefRecord"]["tag"], "type");
    assert!(wire_types["CaptureLeadRequest"]["properties"]
        .get("jobRequest")
        .is_some());
    assert!(wire_types["CaptureLeadRequest"]["properties"]
        .get("contactId")
        .is_some());
    assert!(wire_types["CaptureLeadRequest"]["properties"]
        .get("job_request")
        .is_none());
    assert!(wire_types["WorkQueue"]["properties"]
        .get("localDate")
        .is_some());
    assert!(wire_types["WorkQueueTask"]["properties"]
        .get("dueAt")
        .is_some());
    assert!(wire_types["WorkQueueItem"]["variants"]
        .get("task")
        .and_then(|task| task["properties"].get("linkedRecord"))
        .is_some());
    assert!(wire_types["RecordBrief"]["properties"]
        .get("recordDetailsTruncated")
        .is_some());

    let request = CaptureLeadRequest {
        actor: Actor::Agent,
        name: "Dana Ruiz".into(),
        job_request: "Repair the side gate".into(),
        phone: None,
        email: None,
        note: None,
        contact_id: Some("contact-1".into()),
        next_step_title: Some("Call Dana".into()),
        next_step_due_at: Some("2026-09-08T10:00:00-04:00".into()),
    };
    let request = serde_json::to_value(request).expect("serialize request");
    assert_eq!(request["jobRequest"], "Repair the side gate");
    assert_eq!(request["contactId"], "contact-1");
    assert!(request.get("job_request").is_none());
    assert!(request.get("contact_id").is_none());
}

#[test]
fn captured_lead_queue_and_brief_use_the_documented_discriminators_and_bounds() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = Storage::open_in_app_data(temp.path()).expect("storage");
    let captured = application::capture_lead(
        &mut storage,
        CaptureLeadRequest {
            actor: Actor::User,
            name: "Dana Ruiz".into(),
            job_request: "Repair the side gate".into(),
            phone: None,
            email: None,
            note: None,
            contact_id: None,
            next_step_title: Some("Call Dana".into()),
            next_step_due_at: Some("2026-09-08T10:00:00-04:00".into()),
        },
    )
    .expect("capture lead");
    let captured_value = serde_json::to_value(&captured).expect("serialize capture");
    assert!(captured_value.get("contact").is_some());
    assert!(captured_value.get("opportunity").is_some());
    assert!(captured_value.get("task").is_some());

    let queue = application::get_work_queue(&storage, Some("2026-09-08T08:00:00-04:00".into()))
        .expect("work queue");
    let queue = serde_json::to_value(queue).expect("serialize queue");
    assert_eq!(queue["localDate"], "2026-09-08");
    assert!(queue.get("local_date").is_none());
    let item = &queue["items"][0];
    assert_eq!(item["kind"], "task");
    assert_eq!(item["reason"], "due_today");
    assert_eq!(item["linkedRecord"]["recordType"], "opportunity");
    assert!(item.get("linked_record").is_none());
    assert!(item["task"].get("body").is_none());

    let brief = record_brief::get_record_brief(
        &storage,
        "opportunity",
        &captured.opportunity.id,
        None,
        None,
    )
    .expect("record brief");
    let brief = serde_json::to_value(brief).expect("serialize brief");
    assert_eq!(brief["record"]["type"], "opportunity");
    assert!(brief["record"].get("record").is_some());
    assert!(brief.get("activitiesTruncated").is_some());
    assert!(brief.get("recordDetailsTruncated").is_some());
    assert!(brief.get("activities_truncated").is_none());
}
