//! A small, bounded context packet for an agent preparing one follow-up.
//!
//! It composes existing read use-cases. It does not create a second record
//! model, query attachment bytes, or contact a provider.

use serde::Serialize;

use crate::application::{self, ListTasksRequest, OpportunityDetail};
use crate::attention::AttentionFlag;
use crate::domain::{Activity, Company, Contact, Task};
use crate::error::ApplicationError;
use crate::storage::Storage;

pub const DEFAULT_ACTIVITY_LIMIT: usize = 10;
pub const DEFAULT_TASK_LIMIT: usize = 10;
pub const MAX_ACTIVITY_LIMIT: usize = 25;
pub const MAX_TASK_LIMIT: usize = 25;
pub const MAX_ACTIVITY_BODY_CHARS: usize = 500;
pub const MAX_RECORD_TEXT_CHARS: usize = 500;
pub const MAX_STAGE_HISTORY: usize = 25;
pub const MAX_CONTACT_CHANNELS: usize = 25;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordBrief {
    pub parent_type: String,
    pub parent_id: String,
    pub record: RecordBriefRecord,
    pub linked: LinkedRecords,
    pub activities: Vec<Activity>,
    pub activities_truncated: bool,
    pub activity_text_truncated: bool,
    pub open_tasks: Vec<Task>,
    pub tasks_truncated: bool,
    pub task_text_truncated: bool,
    pub record_text_truncated: bool,
    pub record_details_truncated: bool,
    pub attention_flags: Vec<AttentionFlag>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", content = "record", rename_all = "snake_case")]
pub enum RecordBriefRecord {
    Contact(Contact),
    Company(Company),
    Opportunity(OpportunityDetail),
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedRecords {
    pub contact: Option<Contact>,
    pub company: Option<Company>,
}

/// Build a context packet for one canonical record. Activities include the
/// target's directly related opportunity history, matching `get_timeline`.
pub fn get_record_brief(
    storage: &Storage,
    parent_type: &str,
    parent_id: &str,
    activity_limit: Option<usize>,
    task_limit: Option<usize>,
) -> Result<RecordBrief, ApplicationError> {
    let activity_limit = bounded_limit(
        "activityLimit",
        activity_limit,
        DEFAULT_ACTIVITY_LIMIT,
        MAX_ACTIVITY_LIMIT,
    )?;
    let task_limit = bounded_limit("taskLimit", task_limit, DEFAULT_TASK_LIMIT, MAX_TASK_LIMIT)?;

    let (mut record, mut linked) = match parent_type {
        "contact" => {
            let contact = application::get_contact(storage, parent_id)?;
            let company = match &contact.company_id {
                Some(id) => Some(application::get_company(storage, id)?),
                None => None,
            };
            (
                RecordBriefRecord::Contact(contact),
                LinkedRecords {
                    contact: None,
                    company,
                },
            )
        }
        "company" => (
            RecordBriefRecord::Company(application::get_company(storage, parent_id)?),
            LinkedRecords::default(),
        ),
        "opportunity" => {
            let opportunity = application::get_opportunity(storage, parent_id)?;
            let contact = match &opportunity.opportunity.contact_id {
                Some(id) => Some(application::get_contact(storage, id)?),
                None => None,
            };
            let company = match &opportunity.opportunity.company_id {
                Some(id) => Some(application::get_company(storage, id)?),
                None => None,
            };
            (
                RecordBriefRecord::Opportunity(opportunity),
                LinkedRecords { contact, company },
            )
        }
        _ => {
            return Err(ApplicationError::InvalidInput {
                field: "parentType".into(),
                message: "must be contact, company, or opportunity".into(),
            });
        }
    };

    let mut all_activities = application::get_timeline(storage, parent_type, parent_id, true)?;
    let activities_truncated = all_activities.len() > activity_limit;
    all_activities.truncate(activity_limit);
    let mut activity_text_truncated = false;
    for activity in &mut all_activities {
        activity_text_truncated |= truncate_activity_body(activity);
    }

    let mut all_tasks = application::list_tasks(
        storage,
        ListTasksRequest {
            status: Some("open".into()),
            overdue_only: false,
            parent_type: Some(parent_type.into()),
            parent_id: Some(parent_id.into()),
        },
    )?;
    let tasks_truncated = all_tasks.len() > task_limit;
    all_tasks.truncate(task_limit);
    let task_ids = all_tasks
        .iter()
        .map(|task| task.id.clone())
        .collect::<Vec<_>>();
    let mut task_text_truncated = false;
    for task in &mut all_tasks {
        task_text_truncated |= truncate_task_body(task);
    }

    let attention_flags = application::get_attention_flags(storage, None)?
        .into_iter()
        .filter(|flag| {
            (matches!(
                (parent_type, flag.record_type),
                ("contact", crate::attention::AttentionRecordType::Contact)
                    | (
                        "opportunity",
                        crate::attention::AttentionRecordType::Opportunity
                    )
            ) && flag.record_id == parent_id)
                || (matches!(
                    flag.record_type,
                    crate::attention::AttentionRecordType::Task
                ) && task_ids.contains(&flag.record_id))
        })
        .collect();
    let record_text_truncated = truncate_record(&mut record) | truncate_linked(&mut linked);
    let mut record_details_truncated =
        truncate_record_details(&mut record) | truncate_linked_details(&mut linked);
    record_details_truncated |= if let RecordBriefRecord::Opportunity(detail) = &mut record {
        let cut = detail.stage_history.len() > MAX_STAGE_HISTORY;
        if cut {
            let keep_from = detail.stage_history.len() - MAX_STAGE_HISTORY;
            detail.stage_history.drain(..keep_from);
        }
        cut
    } else {
        false
    };

    Ok(RecordBrief {
        parent_type: parent_type.into(),
        parent_id: parent_id.into(),
        record,
        linked,
        activities: all_activities,
        activities_truncated,
        activity_text_truncated,
        open_tasks: all_tasks,
        tasks_truncated,
        task_text_truncated,
        record_text_truncated,
        record_details_truncated,
        attention_flags,
    })
}

fn bounded_limit(
    field: &str,
    value: Option<usize>,
    default: usize,
    maximum: usize,
) -> Result<usize, ApplicationError> {
    let value = value.unwrap_or(default);
    if value == 0 || value > maximum {
        return Err(ApplicationError::InvalidInput {
            field: field.into(),
            message: format!("must be between 1 and {maximum}"),
        });
    }
    Ok(value)
}

fn truncate_activity_body(activity: &mut Activity) -> bool {
    truncate_text(&mut activity.body, MAX_ACTIVITY_BODY_CHARS)
}

fn truncate_task_body(task: &mut Task) -> bool {
    truncate_text(&mut task.body, MAX_RECORD_TEXT_CHARS)
}
fn truncate_text(value: &mut Option<String>, max: usize) -> bool {
    let Some(text) = value.as_ref() else {
        return false;
    };
    if text.chars().count() <= max {
        return false;
    }
    *value = Some(format!(
        "{}… (truncated)",
        text.chars().take(max).collect::<String>()
    ));
    true
}
fn truncate_contact(contact: &mut Contact) -> bool {
    truncate_text(&mut contact.notes, MAX_RECORD_TEXT_CHARS)
}
fn truncate_company(company: &mut Company) -> bool {
    truncate_text(&mut company.notes, MAX_RECORD_TEXT_CHARS)
        | truncate_text(&mut company.license_notes, MAX_RECORD_TEXT_CHARS)
}
fn truncate_record(record: &mut RecordBriefRecord) -> bool {
    match record {
        RecordBriefRecord::Contact(contact) => truncate_contact(contact),
        RecordBriefRecord::Company(company) => truncate_company(company),
        RecordBriefRecord::Opportunity(detail) => {
            truncate_text(&mut detail.opportunity.notes, MAX_RECORD_TEXT_CHARS)
        }
    }
}
fn truncate_linked(linked: &mut LinkedRecords) -> bool {
    linked
        .contact
        .as_mut()
        .map(truncate_contact)
        .unwrap_or(false)
        | linked
            .company
            .as_mut()
            .map(truncate_company)
            .unwrap_or(false)
}
fn truncate_contact_channels(contact: &mut Contact) -> bool {
    let cut = contact.channels.len() > MAX_CONTACT_CHANNELS;
    contact.channels.truncate(MAX_CONTACT_CHANNELS);
    cut
}
fn truncate_record_details(record: &mut RecordBriefRecord) -> bool {
    match record {
        RecordBriefRecord::Contact(contact) => truncate_contact_channels(contact),
        _ => false,
    }
}
fn truncate_linked_details(linked: &mut LinkedRecords) -> bool {
    linked
        .contact
        .as_mut()
        .map(truncate_contact_channels)
        .unwrap_or(false)
}
