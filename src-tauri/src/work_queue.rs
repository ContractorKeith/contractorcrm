//! Pure work-queue assembly. The application layer supplies already-validated
//! tasks and attention flags; this module only applies the caller's local-day
//! boundary, removes duplicate overdue flags, orders, and caps the result.

use chrono::{DateTime, FixedOffset, Utc};
use serde::Serialize;

use crate::attention::{AttentionFlag, AttentionRecordType, AttentionRule};
use crate::domain::{ParentType, Task, TaskPriority};

/// The queue is deliberately short: it is the next-work surface, not another
/// task report. `truncated` tells callers when they should use Tasks or
/// Attention for the complete list.
pub const MAX_WORK_QUEUE_ITEMS: usize = 50;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkQueue {
    /// The precise RFC3339 instant used to evaluate attention rules.
    pub reference_time: String,
    /// Calendar day in the offset supplied by the caller.
    pub local_date: String,
    pub items: Vec<WorkQueueItem>,
    pub truncated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkQueueRecord {
    pub record_type: ParentType,
    pub record_id: String,
    pub display_name: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkQueueTaskReason {
    Overdue,
    DueToday,
}

/// Only the fields needed to work the queue. Task bodies can be long notes and
/// belong on the record/task view, not in a bounded agent-facing queue.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkQueueTask {
    pub id: String,
    pub title: String,
    pub parent_type: Option<ParentType>,
    pub parent_id: Option<String>,
    pub due_at: String,
    pub priority: TaskPriority,
    pub version: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum WorkQueueItem {
    Task {
        task: WorkQueueTask,
        reason: WorkQueueTaskReason,
        linked_record: Option<WorkQueueRecord>,
    },
    Attention {
        rule: AttentionRule,
        record_type: AttentionRecordType,
        record_id: String,
        record_display_name: String,
        explanation: String,
    },
}

/// Assemble the queue at an explicit local-day boundary. Overdue tasks appear
/// once as actionable task rows; their redundant attention flags are omitted.
pub fn assemble(
    tasks: Vec<Task>,
    flags: Vec<AttentionFlag>,
    reference: DateTime<FixedOffset>,
) -> Result<WorkQueue, String> {
    let reference_utc = reference.with_timezone(&Utc);
    let local_date = reference.date_naive();
    let mut candidates: Vec<(u8, DateTime<Utc>, u8, String, WorkQueueItem)> = Vec::new();

    for task in tasks {
        if task.status.as_database_value() != "open" {
            continue;
        }
        let Some(due_at) = task.due_at.as_deref() else {
            continue;
        };
        let due = DateTime::parse_from_rfc3339(due_at)
            .map_err(|_| format!("task {} has an invalid dueAt timestamp", task.id))?;
        let reason = if due.with_timezone(&Utc) < reference_utc {
            Some(WorkQueueTaskReason::Overdue)
        } else if due.with_timezone(reference.offset()).date_naive() == local_date {
            Some(WorkQueueTaskReason::DueToday)
        } else {
            None
        };
        if let Some(reason) = reason {
            let rank = match reason {
                WorkQueueTaskReason::Overdue => 0,
                WorkQueueTaskReason::DueToday => 1,
            };
            let priority_rank = match task.priority {
                TaskPriority::High => 0,
                TaskPriority::Normal => 1,
                TaskPriority::Low => 2,
            };
            candidates.push((
                rank,
                due.with_timezone(&Utc),
                priority_rank,
                task.id.clone(),
                WorkQueueItem::Task {
                    task: WorkQueueTask {
                        id: task.id,
                        title: task.title,
                        parent_type: task.parent_type,
                        parent_id: task.parent_id,
                        due_at: due_at.to_owned(),
                        priority: task.priority,
                        version: task.version,
                    },
                    reason,
                    linked_record: None,
                },
            ));
        }
    }

    for flag in flags {
        if flag.rule == AttentionRule::OverdueTask {
            continue;
        }
        let rank = match flag.rule {
            AttentionRule::OverdueTask => unreachable!("overdue flags are removed above"),
            AttentionRule::ProposalNoResponse => 2,
            AttentionRule::StaleLead => 3,
        };
        candidates.push((
            rank,
            reference_utc,
            0,
            flag.id.clone(),
            WorkQueueItem::Attention {
                rule: flag.rule,
                record_type: flag.record_type,
                record_id: flag.record_id,
                record_display_name: flag.record_display_name,
                explanation: flag.explanation,
            },
        ));
    }

    candidates.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
            .then_with(|| a.3.cmp(&b.3))
    });
    let truncated = candidates.len() > MAX_WORK_QUEUE_ITEMS;
    let items = candidates
        .into_iter()
        .take(MAX_WORK_QUEUE_ITEMS)
        .map(|(_, _, _, _, item)| item)
        .collect();
    Ok(WorkQueue {
        reference_time: reference.to_rfc3339(),
        local_date: local_date.to_string(),
        items,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{TaskPriority, TaskStatus};

    fn task(id: &str, due_at: &str) -> Task {
        Task {
            id: id.into(),
            title: id.into(),
            body: None,
            parent_type: None,
            parent_id: None,
            due_at: Some(due_at.into()),
            remind_at: None,
            priority: TaskPriority::Normal,
            status: TaskStatus::Open,
            completed_at: None,
            created_at: due_at.into(),
            updated_at: due_at.into(),
            version: 1,
        }
    }

    #[test]
    fn keeps_the_callers_local_day_and_removes_overdue_flags() {
        let reference = DateTime::parse_from_rfc3339("2026-09-08T08:00:00-04:00").unwrap();
        let queue = assemble(
            vec![
                task("overdue", "2026-09-08T07:00:00-04:00"),
                task("today", "2026-09-08T23:00:00-04:00"),
                task("tomorrow", "2026-09-09T00:30:00-04:00"),
            ],
            vec![AttentionFlag {
                id: "overdue_task:overdue".into(),
                rule: AttentionRule::OverdueTask,
                record_type: AttentionRecordType::Task,
                record_id: "overdue".into(),
                record_display_name: "overdue".into(),
                explanation: "duplicate".into(),
            }],
            reference,
        )
        .unwrap();
        assert_eq!(queue.local_date, "2026-09-08");
        assert_eq!(queue.items.len(), 2);
        assert!(matches!(
            queue.items[0],
            WorkQueueItem::Task {
                reason: WorkQueueTaskReason::Overdue,
                ..
            }
        ));
        assert!(matches!(
            queue.items[1],
            WorkQueueItem::Task {
                reason: WorkQueueTaskReason::DueToday,
                ..
            }
        ));
    }

    #[test]
    fn sorts_by_instant_then_priority_and_uses_camel_case_fields() {
        let reference = DateTime::parse_from_rfc3339("2026-09-08T08:00:00-04:00").unwrap();
        let mut normal = task("normal", "2026-09-08T09:00:00-04:00");
        normal.priority = TaskPriority::Normal;
        let mut high = task("high", "2026-09-08T13:00:00Z"); // same instant as normal
        high.priority = TaskPriority::High;
        let early = task("early", "2026-09-08T08:30:00-04:00");
        let queue = assemble(vec![normal, high, early], vec![], reference).unwrap();
        let ids = queue
            .items
            .iter()
            .map(|item| match item {
                WorkQueueItem::Task { task, .. } => task.id.as_str(),
                _ => unreachable!(),
            })
            .collect::<Vec<_>>();
        assert_eq!(ids, ["early", "high", "normal"]);
        let serialized = serde_json::to_value(queue).unwrap();
        assert!(serialized["items"][0]["linkedRecord"].is_null());
        assert!(serialized["items"][0]["task"]["dueAt"].is_string());
        assert!(serialized["items"][0]["task"].get("due_at").is_none());
    }
}
