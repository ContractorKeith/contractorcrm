use contractorcrm_lib::application::{
    self, CaptureLeadRequest, ContactPatch, CreateContactRequest,
};
use contractorcrm_lib::domain::Actor;
use contractorcrm_lib::storage::Storage;

fn open_storage(temp: &tempfile::TempDir) -> Storage {
    Storage::open_in_app_data(temp.path()).unwrap()
}

#[test]
fn capture_lead_creates_linked_records_and_indexes_them_together() {
    let temp = tempfile::tempdir().unwrap();
    let mut storage = open_storage(&temp);
    let captured = application::capture_lead(
        &mut storage,
        CaptureLeadRequest {
            actor: Actor::User,
            name: "Dana Ruiz".into(),
            job_request: "Replace side gate".into(),
            phone: Some("555-0100".into()),
            email: None,
            note: Some("Requested black aluminum.".into()),
            contact_id: None,
            next_step_title: Some("Call Dana".into()),
            next_step_due_at: Some("2026-09-10T14:00:00.000Z".into()),
        },
    )
    .unwrap();
    assert_eq!(
        captured.contact.kind,
        contractorcrm_lib::domain::PartyKind::Lead
    );
    assert_eq!(
        captured.opportunity.contact_id.as_deref(),
        Some(captured.contact.id.as_str())
    );
    assert_eq!(
        captured.task.as_ref().unwrap().parent_id.as_deref(),
        Some(captured.opportunity.id.as_str())
    );
    assert!(application::search_records(
        &storage,
        "side gate".into(),
        Some(vec!["opportunity".into()]),
        None
    )
    .unwrap()
    .iter()
    .any(|hit| hit.entity_id == captured.opportunity.id));
}

#[test]
fn capture_lead_rolls_back_a_created_contact_opportunity_and_audit_rows_when_the_task_insert_fails()
{
    let temp = tempfile::tempdir().unwrap();
    let mut storage = open_storage(&temp);
    storage.connection().execute_batch("CREATE TRIGGER fail_captured_task BEFORE INSERT ON tasks BEGIN SELECT RAISE(ABORT, 'task blocked'); END;").unwrap();
    let result = application::capture_lead(
        &mut storage,
        CaptureLeadRequest {
            actor: Actor::User,
            name: "Dana Ruiz".into(),
            job_request: "Fence repair".into(),
            phone: None,
            email: None,
            note: None,
            contact_id: None,
            next_step_title: Some("Call Dana".into()),
            next_step_due_at: None,
        },
    );
    assert!(result.is_err());
    assert!(application::list_contacts(&storage, false)
        .unwrap()
        .is_empty());
    assert!(application::list_opportunities(&storage, false)
        .unwrap()
        .is_empty());
    let capture_logs: i64 = storage
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM command_log WHERE summary LIKE 'captured lead%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        capture_logs, 0,
        "a failed capture must not leave audit rows"
    );
}

#[test]
fn capture_lead_links_an_existing_contact_without_changing_it() {
    let temp = tempfile::tempdir().unwrap();
    let mut storage = open_storage(&temp);
    let contact = application::create_contact(
        &mut storage,
        CreateContactRequest {
            actor: Actor::User,
            contact: ContactPatch {
                display_name: Some("Dana Ruiz".into()),
                kind: "client".into(),
                ..ContactPatch::default()
            },
        },
    )
    .unwrap();
    let captured = application::capture_lead(
        &mut storage,
        CaptureLeadRequest {
            actor: Actor::User,
            name: "Dana Ruiz".into(),
            job_request: "Fence repair".into(),
            phone: None,
            email: None,
            note: None,
            contact_id: Some(contact.id.clone()),
            next_step_title: None,
            next_step_due_at: None,
        },
    )
    .unwrap();
    assert_eq!(captured.contact.id, contact.id);
    assert_eq!(
        application::get_contact(&storage, &contact.id)
            .unwrap()
            .version,
        1
    );
}
