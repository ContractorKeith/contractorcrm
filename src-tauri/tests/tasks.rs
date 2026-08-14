//! Integration tests for the tasks domain — personal and parented tasks,
//! the both-or-neither parent rule, complete with optional activity logging,
//! reopen/drop, the overdue filter, list ordering, version conflicts, hard
//! delete, and command_log rows.

use contractorcrm_lib::application::{
    complete_task, create_contact, create_task, delete_task, drop_task, get_timeline, list_tasks,
    reopen_task, update_task, CompleteTaskRequest, ContactPatch, CreateContactRequest,
    CreateTaskRequest, ListTasksRequest, TaskActionRequest, TaskPatch, UpdateTaskRequest,
};
use contractorcrm_lib::domain::{
    ActivityDirection, ActivityKind, Actor, Contact, ParentType, Task, TaskPriority, TaskStatus,
};
use contractorcrm_lib::error::ApplicationError;
use contractorcrm_lib::storage::{new_id, now_utc, Storage};
use rusqlite::params;

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

fn command_log_summaries(storage: &Storage, entity_type: &str, entity_id: &str) -> Vec<String> {
    let mut statement = storage
        .connection()
        .prepare(
            "SELECT summary FROM command_log
             WHERE entity_type = ?1 AND entity_id = ?2 ORDER BY created_at, id",
        )
        .expect("prepare command_log query");
    statement
        .query_map([entity_type, entity_id], |row| row.get(0))
        .expect("query command_log")
        .collect::<rusqlite::Result<Vec<String>>>()
        .expect("collect command_log summaries")
}

// ---------------------------------------------------------------------------
// Create — personal and parented, parent rules
// ---------------------------------------------------------------------------

#[test]
fn creates_a_personal_and_a_parented_task() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Homeowner");

    let personal = make_task(&mut storage, "Order truck tires", TaskPatch::default());
    assert_eq!(personal.parent_type, None);
    assert_eq!(personal.parent_id, None);
    assert_eq!(personal.priority, TaskPriority::Normal);
    assert_eq!(personal.status, TaskStatus::Open);
    assert_eq!(personal.completed_at, None);
    assert_eq!(personal.version, 1);

    let parented = make_task(
        &mut storage,
        "Call Dana about the gate",
        TaskPatch {
            parent_type: Some("contact".into()),
            parent_id: Some(contact.id.clone()),
            due_at: Some("2026-08-20T12:00:00.000Z".into()),
            remind_at: Some("2026-08-19T12:00:00.000Z".into()),
            priority: Some("high".into()),
            body: Some("She asked for a **self-closing** hinge.".into()),
            ..TaskPatch::default()
        },
    );
    assert_eq!(parented.parent_type, Some(ParentType::Contact));
    assert_eq!(parented.parent_id.as_deref(), Some(contact.id.as_str()));
    assert_eq!(parented.due_at.as_deref(), Some("2026-08-20T12:00:00.000Z"));
    assert_eq!(
        parented.remind_at.as_deref(),
        Some("2026-08-19T12:00:00.000Z")
    );
    assert_eq!(parented.priority, TaskPriority::High);
}

#[test]
fn parent_type_and_id_must_be_set_together() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Homeowner");

    // Application-level rule: type without id fails, and id without type fails.
    for (parent_type, parent_id) in [
        (Some("contact".to_string()), None),
        (None, Some(contact.id.clone())),
    ] {
        let error = create_task(
            &mut storage,
            CreateTaskRequest {
                actor: Actor::User,
                task: TaskPatch {
                    title: "Half a parent".into(),
                    parent_type,
                    parent_id,
                    ..TaskPatch::default()
                },
            },
        )
        .expect_err("half-set parent must fail");
        assert_eq!(error.kind(), "invalid_input");
    }

    // The database CHECK backstops the same rule against raw writes.
    let now = now_utc();
    let raw = storage.connection().execute(
        "INSERT INTO tasks (id, title, parent_type, parent_id, created_at, updated_at)
         VALUES (?1, 'Raw half parent', 'contact', NULL, ?2, ?2)",
        params![new_id(), now],
    );
    assert!(raw.is_err(), "DB CHECK must reject a half-set parent");
}

#[test]
fn creating_on_a_missing_parent_is_not_found() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);

    let error = create_task(
        &mut storage,
        CreateTaskRequest {
            actor: Actor::User,
            task: TaskPatch {
                title: "Task for nobody".into(),
                parent_type: Some("contact".into()),
                parent_id: Some("missing-contact".into()),
                ..TaskPatch::default()
            },
        },
    )
    .expect_err("missing parent must fail");
    assert_eq!(error.kind(), "not_found");
}

// ---------------------------------------------------------------------------
// Complete — with and without activity logging
// ---------------------------------------------------------------------------

#[test]
fn complete_without_log_sets_done_and_completed_at() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Homeowner");
    let task = make_task(
        &mut storage,
        "Call Dana",
        TaskPatch {
            parent_type: Some("contact".into()),
            parent_id: Some(contact.id.clone()),
            ..TaskPatch::default()
        },
    );

    let done = complete_task(
        &mut storage,
        CompleteTaskRequest {
            actor: Actor::User,
            task_id: task.id.clone(),
            expected_version: 1,
            log_activity: false,
        },
    )
    .expect("complete task");
    assert_eq!(done.status, TaskStatus::Done);
    assert!(done.completed_at.is_some());
    assert_eq!(done.version, 2);

    // No activity was written on the parent.
    let timeline = get_timeline(&storage, "contact", &contact.id, false).expect("timeline");
    assert!(timeline.is_empty());

    // Completing again is rejected — the task is no longer open.
    let again = complete_task(
        &mut storage,
        CompleteTaskRequest {
            actor: Actor::User,
            task_id: task.id.clone(),
            expected_version: 2,
            log_activity: false,
        },
    )
    .expect_err("double complete must fail");
    assert_eq!(again.kind(), "validation_failed");
}

#[test]
fn complete_with_log_writes_the_activity_in_the_same_transaction() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Homeowner");
    let task = make_task(
        &mut storage,
        "Send the estimate",
        TaskPatch {
            parent_type: Some("contact".into()),
            parent_id: Some(contact.id.clone()),
            ..TaskPatch::default()
        },
    );

    let done = complete_task(
        &mut storage,
        CompleteTaskRequest {
            actor: Actor::User,
            task_id: task.id.clone(),
            expected_version: 1,
            log_activity: true,
        },
    )
    .expect("complete with log");
    assert_eq!(done.status, TaskStatus::Done);

    // The note lands on the task's parent and shows in the timeline.
    let timeline = get_timeline(&storage, "contact", &contact.id, false).expect("timeline");
    assert_eq!(timeline.len(), 1);
    let note = &timeline[0];
    assert_eq!(note.kind, ActivityKind::Note);
    assert_eq!(note.direction, ActivityDirection::None);
    assert_eq!(note.summary, "Completed task: Send the estimate");
    assert_eq!(note.parent_type, ParentType::Contact);
    assert_eq!(note.parent_id, contact.id);
}

#[test]
fn complete_with_log_on_a_parentless_task_is_invalid_and_writes_nothing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let task = make_task(&mut storage, "Personal errand", TaskPatch::default());

    let error = complete_task(
        &mut storage,
        CompleteTaskRequest {
            actor: Actor::User,
            task_id: task.id.clone(),
            expected_version: 1,
            log_activity: true,
        },
    )
    .expect_err("parentless log must fail");
    assert_eq!(error.kind(), "invalid_input");

    // The whole transaction rolled back — the task is still open at version 1.
    let tasks = list_tasks(&storage, ListTasksRequest::default()).expect("list tasks");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].status, TaskStatus::Open);
    assert_eq!(tasks[0].version, 1);
    assert_eq!(tasks[0].completed_at, None);
}

// ---------------------------------------------------------------------------
// Reopen and drop
// ---------------------------------------------------------------------------

#[test]
fn reopen_clears_completed_at_from_done_and_dropped_tasks() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let task = make_task(&mut storage, "Flaky follow-up", TaskPatch::default());

    let done = complete_task(
        &mut storage,
        CompleteTaskRequest {
            actor: Actor::User,
            task_id: task.id.clone(),
            expected_version: 1,
            log_activity: false,
        },
    )
    .expect("complete");
    assert!(done.completed_at.is_some());

    let reopened = reopen_task(
        &mut storage,
        TaskActionRequest {
            actor: Actor::User,
            task_id: task.id.clone(),
            expected_version: 2,
        },
    )
    .expect("reopen done task");
    assert_eq!(reopened.status, TaskStatus::Open);
    assert_eq!(reopened.completed_at, None);
    assert_eq!(reopened.version, 3);

    // Reopening an already-open task is rejected.
    let already_open = reopen_task(
        &mut storage,
        TaskActionRequest {
            actor: Actor::User,
            task_id: task.id.clone(),
            expected_version: 3,
        },
    )
    .expect_err("reopen open task must fail");
    assert_eq!(already_open.kind(), "validation_failed");

    // Drop, then reopen from dropped as well.
    let dropped = drop_task(
        &mut storage,
        TaskActionRequest {
            actor: Actor::User,
            task_id: task.id.clone(),
            expected_version: 3,
        },
    )
    .expect("drop task");
    assert_eq!(dropped.status, TaskStatus::Dropped);
    assert_eq!(dropped.completed_at, None);

    let back = reopen_task(
        &mut storage,
        TaskActionRequest {
            actor: Actor::User,
            task_id: task.id.clone(),
            expected_version: 4,
        },
    )
    .expect("reopen dropped task");
    assert_eq!(back.status, TaskStatus::Open);
    assert_eq!(back.completed_at, None);
}

// ---------------------------------------------------------------------------
// Update, list filters, and ordering
// ---------------------------------------------------------------------------

#[test]
fn update_replaces_editable_fields_but_never_status() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Homeowner");
    let task = make_task(&mut storage, "Rough draft", TaskPatch::default());

    let updated = update_task(
        &mut storage,
        UpdateTaskRequest {
            actor: Actor::User,
            task_id: task.id.clone(),
            expected_version: 1,
            patch: TaskPatch {
                title: "Final follow-up".into(),
                body: Some("Bring the samples.".into()),
                parent_type: Some("contact".into()),
                parent_id: Some(contact.id.clone()),
                due_at: Some("2026-08-21T12:00:00.000Z".into()),
                remind_at: None,
                priority: Some("low".into()),
            },
        },
    )
    .expect("update task");
    assert_eq!(updated.title, "Final follow-up");
    assert_eq!(updated.parent_type, Some(ParentType::Contact));
    assert_eq!(updated.priority, TaskPriority::Low);
    assert_eq!(updated.status, TaskStatus::Open);
    assert_eq!(updated.version, 2);
}

#[test]
fn overdue_filter_returns_only_open_tasks_past_due() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);

    let past = make_task(
        &mut storage,
        "Past due",
        TaskPatch {
            due_at: Some("2020-01-01T00:00:00.000Z".into()),
            ..TaskPatch::default()
        },
    );
    make_task(
        &mut storage,
        "Future due",
        TaskPatch {
            due_at: Some("2099-01-01T00:00:00.000Z".into()),
            ..TaskPatch::default()
        },
    );
    make_task(&mut storage, "No due date", TaskPatch::default());
    // A past-due but completed task is not overdue.
    let done_past = make_task(
        &mut storage,
        "Done and past due",
        TaskPatch {
            due_at: Some("2020-06-01T00:00:00.000Z".into()),
            ..TaskPatch::default()
        },
    );
    complete_task(
        &mut storage,
        CompleteTaskRequest {
            actor: Actor::User,
            task_id: done_past.id.clone(),
            expected_version: 1,
            log_activity: false,
        },
    )
    .expect("complete");

    let overdue = list_tasks(
        &storage,
        ListTasksRequest {
            overdue_only: true,
            ..ListTasksRequest::default()
        },
    )
    .expect("list overdue");
    let titles: Vec<&str> = overdue.iter().map(|t| t.title.as_str()).collect();
    assert_eq!(titles, vec!["Past due"]);
    assert_eq!(overdue[0].id, past.id);

    // Status filter: only done tasks.
    let done = list_tasks(
        &storage,
        ListTasksRequest {
            status: Some("done".into()),
            ..ListTasksRequest::default()
        },
    )
    .expect("list done");
    let titles: Vec<&str> = done.iter().map(|t| t.title.as_str()).collect();
    assert_eq!(titles, vec!["Done and past due"]);
}

#[test]
fn list_orders_by_due_date_nulls_last_then_priority() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Homeowner");

    make_task(&mut storage, "No due, normal", TaskPatch::default());
    make_task(
        &mut storage,
        "No due, high",
        TaskPatch {
            priority: Some("high".into()),
            ..TaskPatch::default()
        },
    );
    make_task(
        &mut storage,
        "Later due",
        TaskPatch {
            due_at: Some("2026-09-01T00:00:00.000Z".into()),
            ..TaskPatch::default()
        },
    );
    // Same due date — priority breaks the tie, high first.
    make_task(
        &mut storage,
        "Sooner due, low",
        TaskPatch {
            due_at: Some("2026-08-20T00:00:00.000Z".into()),
            priority: Some("low".into()),
            ..TaskPatch::default()
        },
    );
    make_task(
        &mut storage,
        "Sooner due, high",
        TaskPatch {
            due_at: Some("2026-08-20T00:00:00.000Z".into()),
            priority: Some("high".into()),
            parent_type: Some("contact".into()),
            parent_id: Some(contact.id.clone()),
            ..TaskPatch::default()
        },
    );

    let tasks = list_tasks(&storage, ListTasksRequest::default()).expect("list tasks");
    let titles: Vec<&str> = tasks.iter().map(|t| t.title.as_str()).collect();
    assert_eq!(
        titles,
        vec![
            "Sooner due, high",
            "Sooner due, low",
            "Later due",
            "No due, high",
            "No due, normal",
        ]
    );

    // Parent filter narrows to that record's tasks.
    let for_contact = list_tasks(
        &storage,
        ListTasksRequest {
            parent_type: Some("contact".into()),
            parent_id: Some(contact.id.clone()),
            ..ListTasksRequest::default()
        },
    )
    .expect("list contact tasks");
    let titles: Vec<&str> = for_contact.iter().map(|t| t.title.as_str()).collect();
    assert_eq!(titles, vec!["Sooner due, high"]);
}

// ---------------------------------------------------------------------------
// Versions, delete, and the command log
// ---------------------------------------------------------------------------

#[test]
fn stale_versions_conflict_on_every_mutation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let task = make_task(&mut storage, "Versioned", TaskPatch::default());

    let stale_update = update_task(
        &mut storage,
        UpdateTaskRequest {
            actor: Actor::User,
            task_id: task.id.clone(),
            expected_version: 9,
            patch: TaskPatch {
                title: "Too late".into(),
                ..TaskPatch::default()
            },
        },
    )
    .expect_err("stale update must conflict");
    assert!(matches!(
        stale_update,
        ApplicationError::VersionConflict {
            expected: 9,
            current: 1,
            ..
        }
    ));
    assert_eq!(stale_update.kind(), "version_conflict");

    for result in [
        complete_task(
            &mut storage,
            CompleteTaskRequest {
                actor: Actor::User,
                task_id: task.id.clone(),
                expected_version: 9,
                log_activity: false,
            },
        )
        .map(|_| ()),
        drop_task(
            &mut storage,
            TaskActionRequest {
                actor: Actor::User,
                task_id: task.id.clone(),
                expected_version: 9,
            },
        )
        .map(|_| ()),
        delete_task(
            &mut storage,
            TaskActionRequest {
                actor: Actor::User,
                task_id: task.id.clone(),
                expected_version: 9,
            },
        ),
    ] {
        assert_eq!(
            result.expect_err("stale mutation must conflict").kind(),
            "version_conflict"
        );
    }
}

#[test]
fn delete_removes_the_task_and_every_mutation_logs_a_command() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Homeowner");
    let task = make_task(
        &mut storage,
        "Full lifecycle",
        TaskPatch {
            parent_type: Some("contact".into()),
            parent_id: Some(contact.id.clone()),
            ..TaskPatch::default()
        },
    );

    update_task(
        &mut storage,
        UpdateTaskRequest {
            actor: Actor::Agent,
            task_id: task.id.clone(),
            expected_version: 1,
            patch: TaskPatch {
                title: "Full lifecycle (renamed)".into(),
                parent_type: Some("contact".into()),
                parent_id: Some(contact.id.clone()),
                ..TaskPatch::default()
            },
        },
    )
    .expect("update");
    complete_task(
        &mut storage,
        CompleteTaskRequest {
            actor: Actor::User,
            task_id: task.id.clone(),
            expected_version: 2,
            log_activity: true,
        },
    )
    .expect("complete with log");
    reopen_task(
        &mut storage,
        TaskActionRequest {
            actor: Actor::User,
            task_id: task.id.clone(),
            expected_version: 3,
        },
    )
    .expect("reopen");
    drop_task(
        &mut storage,
        TaskActionRequest {
            actor: Actor::User,
            task_id: task.id.clone(),
            expected_version: 4,
        },
    )
    .expect("drop");
    delete_task(
        &mut storage,
        TaskActionRequest {
            actor: Actor::User,
            task_id: task.id.clone(),
            expected_version: 5,
        },
    )
    .expect("delete");

    let tasks = list_tasks(&storage, ListTasksRequest::default()).expect("list tasks");
    assert!(tasks.is_empty(), "deleted task must be gone");

    let summaries = command_log_summaries(&storage, "task", &task.id);
    assert_eq!(
        summaries,
        vec![
            "created task \"Full lifecycle\"",
            "updated task \"Full lifecycle (renamed)\"",
            "completed task \"Full lifecycle (renamed)\"",
            "reopened task \"Full lifecycle (renamed)\"",
            "dropped task \"Full lifecycle (renamed)\"",
            "deleted task \"Full lifecycle (renamed)\"",
        ]
    );

    // The logged-on-complete note wrote its own activity command row, and the
    // note itself survives the task's deletion.
    let timeline = get_timeline(&storage, "contact", &contact.id, false).expect("timeline");
    assert_eq!(timeline.len(), 1);
    let activity_summaries = command_log_summaries(&storage, "activity", &timeline[0].id);
    assert_eq!(
        activity_summaries,
        vec!["logged note activity \"Completed task: Full lifecycle (renamed)\""]
    );
}
