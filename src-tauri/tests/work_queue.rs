use contractorcrm_lib::application::{
    create_contact, create_task, get_work_queue, ContactPatch, CreateContactRequest,
    CreateTaskRequest, TaskPatch,
};
use contractorcrm_lib::domain::Actor;
use contractorcrm_lib::storage::Storage;
use contractorcrm_lib::work_queue::{WorkQueueItem, WorkQueueTaskReason};

fn storage() -> (tempfile::TempDir, Storage) {
    let temp = tempfile::tempdir().expect("tempdir");
    let storage = Storage::open_in_app_data(temp.path()).expect("storage");
    (temp, storage)
}

#[test]
fn queue_uses_the_supplied_offset_and_keeps_an_overdue_task_once() {
    let (_temp, mut storage) = storage();
    let contact = create_contact(
        &mut storage,
        CreateContactRequest {
            actor: Actor::User,
            contact: ContactPatch {
                display_name: Some("Dana Ruiz".into()),
                kind: "lead".into(),
                ..ContactPatch::default()
            },
        },
    )
    .expect("contact");
    create_task(
        &mut storage,
        CreateTaskRequest {
            actor: Actor::User,
            task: TaskPatch {
                title: "Past due call".into(),
                parent_type: Some("contact".into()),
                parent_id: Some(contact.id.clone()),
                due_at: Some("2026-09-08T07:00:00-04:00".into()),
                ..TaskPatch::default()
            },
        },
    )
    .expect("overdue task");
    create_task(
        &mut storage,
        CreateTaskRequest {
            actor: Actor::User,
            task: TaskPatch {
                title: "Later today".into(),
                due_at: Some("2026-09-08T23:00:00-04:00".into()),
                ..TaskPatch::default()
            },
        },
    )
    .expect("today task");
    create_task(
        &mut storage,
        CreateTaskRequest {
            actor: Actor::User,
            task: TaskPatch {
                title: "Tomorrow".into(),
                due_at: Some("2026-09-09T00:30:00-04:00".into()),
                ..TaskPatch::default()
            },
        },
    )
    .expect("tomorrow task");

    let queue = get_work_queue(&storage, Some("2026-09-08T08:00:00-04:00".into())).expect("queue");
    assert_eq!(queue.local_date, "2026-09-08");
    assert_eq!(
        queue.items.len(),
        2,
        "the task row replaces its overdue flag"
    );
    let WorkQueueItem::Task {
        task,
        reason,
        linked_record,
    } = &queue.items[0]
    else {
        panic!("first row task")
    };
    assert_eq!(task.title, "Past due call");
    assert_eq!(*reason, WorkQueueTaskReason::Overdue);
    assert_eq!(
        linked_record
            .as_ref()
            .map(|record| record.display_name.as_str()),
        Some("Dana Ruiz")
    );
    assert!(matches!(
        queue.items[1],
        WorkQueueItem::Task {
            reason: WorkQueueTaskReason::DueToday,
            ..
        }
    ));
}

#[test]
fn queue_rejects_a_reference_without_an_offset() {
    let (_temp, storage) = storage();
    let error =
        get_work_queue(&storage, Some("2026-09-08T08:00:00".into())).expect_err("offset required");
    assert!(error.to_string().contains("referenceTime"));
}
