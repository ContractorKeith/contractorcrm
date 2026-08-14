pub mod application;
pub mod attention;
pub mod domain;
pub mod error;
pub mod storage;

use std::sync::Mutex;

use application::{
    ArchiveRequest, CompleteTaskRequest, ContactListItem, CreateCompanyRequest,
    CreateContactRequest, CreateOpportunityRequest, CreateTaskRequest, DatabaseInfo,
    DeleteActivityRequest, EnvelopeExportReport, LinkJobRequest, LinkQuoteRequest,
    ListTasksRequest, LogActivityRequest, MoveOpportunityStageRequest, OpportunityDetail,
    OpportunityListItem, RestoreReport, SetAttentionThresholdsRequest, TaskActionRequest,
    UnlinkHandoffRequest, UpdateActivityRequest, UpdateCompanyRequest, UpdateContactRequest,
    UpdateOpportunityRequest, UpdateStageRequest, UpdateTaskRequest,
};
use attention::{AttentionFlag, Thresholds};
use domain::{Activity, Company, Contact, LostReason, Opportunity, Stage, Task};
use error::ApplicationError;
use serde::Serialize;
use storage::Storage;
use tauri::{Manager, State};

/// Managed application state — one storage handle behind a mutex because the
/// SQLite connection is Send but not Sync.
type SharedStorage = Mutex<Storage>;

/// Wire shape for application errors — stable kind plus per-kind details.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CommandError {
    kind: &'static str,
    message: String,
    #[serde(flatten)]
    details: Box<CommandErrorDetails>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all_fields = "camelCase", untagged)]
enum CommandErrorDetails {
    InvalidInput {
        field: String,
    },
    Validation {
        code: &'static str,
        field: String,
    },
    Record {
        resource: &'static str,
        record_id: String,
    },
    VersionConflict {
        resource: &'static str,
        record_id: String,
        expected_version: i64,
        current_version: i64,
    },
    None {},
}

impl From<ApplicationError> for CommandError {
    fn from(error: ApplicationError) -> Self {
        let details = match &error {
            ApplicationError::InvalidInput { field, .. } => CommandErrorDetails::InvalidInput {
                field: field.clone(),
            },
            ApplicationError::ValidationFailed { code, field, .. } => {
                CommandErrorDetails::Validation {
                    code,
                    field: field.clone(),
                }
            }
            ApplicationError::NotFound { resource, id } => CommandErrorDetails::Record {
                resource,
                record_id: id.clone(),
            },
            ApplicationError::MissingLostReason { id } => CommandErrorDetails::Record {
                resource: "opportunity",
                record_id: id.clone(),
            },
            ApplicationError::VersionConflict {
                resource,
                id,
                expected,
                current,
            } => CommandErrorDetails::VersionConflict {
                resource,
                record_id: id.clone(),
                expected_version: *expected,
                current_version: *current,
            },
            ApplicationError::InvalidStoredData(_)
            | ApplicationError::BackupFailed(_)
            | ApplicationError::RestoreInvalid(_)
            | ApplicationError::Database(_)
            | ApplicationError::Io(_) => CommandErrorDetails::None {},
        };
        Self {
            kind: error.kind(),
            message: error.to_string(),
            details: Box::new(details),
        }
    }
}

/// Report returned by the `health` command — proves the UI → Rust seam works.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HealthReport {
    pub app: &'static str,
    pub version: &'static str,
    pub status: &'static str,
}

/// Pure health logic, kept separate from the Tauri command so it is testable.
fn health_report() -> HealthReport {
    HealthReport {
        app: "ContractorCRM",
        version: env!("CARGO_PKG_VERSION"),
        status: "ok",
    }
}

/// Health check command invoked from the UI over the application seam.
#[tauri::command]
fn health() -> HealthReport {
    health_report()
}

// Thin Tauri commands — lock the shared storage, delegate to the application
// layer, and translate errors to the wire shape.

#[tauri::command]
fn create_company(
    storage: State<'_, SharedStorage>,
    request: CreateCompanyRequest,
) -> Result<Company, CommandError> {
    let mut storage = storage.lock().expect("storage mutex poisoned");
    application::create_company(&mut storage, request).map_err(Into::into)
}

#[tauri::command]
fn update_company(
    storage: State<'_, SharedStorage>,
    request: UpdateCompanyRequest,
) -> Result<Company, CommandError> {
    let mut storage = storage.lock().expect("storage mutex poisoned");
    application::update_company(&mut storage, request).map_err(Into::into)
}

#[tauri::command]
fn archive_company(
    storage: State<'_, SharedStorage>,
    request: ArchiveRequest,
) -> Result<Company, CommandError> {
    let mut storage = storage.lock().expect("storage mutex poisoned");
    application::archive_company(&mut storage, request).map_err(Into::into)
}

#[tauri::command]
fn unarchive_company(
    storage: State<'_, SharedStorage>,
    request: ArchiveRequest,
) -> Result<Company, CommandError> {
    let mut storage = storage.lock().expect("storage mutex poisoned");
    application::unarchive_company(&mut storage, request).map_err(Into::into)
}

#[tauri::command]
fn list_companies(
    storage: State<'_, SharedStorage>,
    include_archived: bool,
) -> Result<Vec<Company>, CommandError> {
    let storage = storage.lock().expect("storage mutex poisoned");
    application::list_companies(&storage, include_archived).map_err(Into::into)
}

#[tauri::command]
fn get_company(
    storage: State<'_, SharedStorage>,
    company_id: String,
) -> Result<Company, CommandError> {
    let storage = storage.lock().expect("storage mutex poisoned");
    application::get_company(&storage, &company_id).map_err(Into::into)
}

#[tauri::command]
fn create_contact(
    storage: State<'_, SharedStorage>,
    request: CreateContactRequest,
) -> Result<Contact, CommandError> {
    let mut storage = storage.lock().expect("storage mutex poisoned");
    application::create_contact(&mut storage, request).map_err(Into::into)
}

#[tauri::command]
fn update_contact(
    storage: State<'_, SharedStorage>,
    request: UpdateContactRequest,
) -> Result<Contact, CommandError> {
    let mut storage = storage.lock().expect("storage mutex poisoned");
    application::update_contact(&mut storage, request).map_err(Into::into)
}

#[tauri::command]
fn archive_contact(
    storage: State<'_, SharedStorage>,
    request: ArchiveRequest,
) -> Result<Contact, CommandError> {
    let mut storage = storage.lock().expect("storage mutex poisoned");
    application::archive_contact(&mut storage, request).map_err(Into::into)
}

#[tauri::command]
fn unarchive_contact(
    storage: State<'_, SharedStorage>,
    request: ArchiveRequest,
) -> Result<Contact, CommandError> {
    let mut storage = storage.lock().expect("storage mutex poisoned");
    application::unarchive_contact(&mut storage, request).map_err(Into::into)
}

#[tauri::command]
fn list_contacts(
    storage: State<'_, SharedStorage>,
    include_archived: bool,
) -> Result<Vec<ContactListItem>, CommandError> {
    let storage = storage.lock().expect("storage mutex poisoned");
    application::list_contacts(&storage, include_archived).map_err(Into::into)
}

#[tauri::command]
fn get_contact(
    storage: State<'_, SharedStorage>,
    contact_id: String,
) -> Result<Contact, CommandError> {
    let storage = storage.lock().expect("storage mutex poisoned");
    application::get_contact(&storage, &contact_id).map_err(Into::into)
}

// Pipeline commands — stages, lost reasons, opportunities, stage moves.

#[tauri::command]
fn list_stages(storage: State<'_, SharedStorage>) -> Result<Vec<Stage>, CommandError> {
    let storage = storage.lock().expect("storage mutex poisoned");
    application::list_stages(&storage).map_err(Into::into)
}

#[tauri::command]
fn update_stage(
    storage: State<'_, SharedStorage>,
    request: UpdateStageRequest,
) -> Result<Stage, CommandError> {
    let mut storage = storage.lock().expect("storage mutex poisoned");
    application::update_stage(&mut storage, request).map_err(Into::into)
}

#[tauri::command]
fn list_lost_reasons(storage: State<'_, SharedStorage>) -> Result<Vec<LostReason>, CommandError> {
    let storage = storage.lock().expect("storage mutex poisoned");
    application::list_lost_reasons(&storage).map_err(Into::into)
}

#[tauri::command]
fn create_opportunity(
    storage: State<'_, SharedStorage>,
    request: CreateOpportunityRequest,
) -> Result<Opportunity, CommandError> {
    let mut storage = storage.lock().expect("storage mutex poisoned");
    application::create_opportunity(&mut storage, request).map_err(Into::into)
}

#[tauri::command]
fn update_opportunity(
    storage: State<'_, SharedStorage>,
    request: UpdateOpportunityRequest,
) -> Result<Opportunity, CommandError> {
    let mut storage = storage.lock().expect("storage mutex poisoned");
    application::update_opportunity(&mut storage, request).map_err(Into::into)
}

#[tauri::command]
fn archive_opportunity(
    storage: State<'_, SharedStorage>,
    request: ArchiveRequest,
) -> Result<Opportunity, CommandError> {
    let mut storage = storage.lock().expect("storage mutex poisoned");
    application::archive_opportunity(&mut storage, request).map_err(Into::into)
}

#[tauri::command]
fn unarchive_opportunity(
    storage: State<'_, SharedStorage>,
    request: ArchiveRequest,
) -> Result<Opportunity, CommandError> {
    let mut storage = storage.lock().expect("storage mutex poisoned");
    application::unarchive_opportunity(&mut storage, request).map_err(Into::into)
}

#[tauri::command]
fn list_opportunities(
    storage: State<'_, SharedStorage>,
    include_archived: bool,
) -> Result<Vec<OpportunityListItem>, CommandError> {
    let storage = storage.lock().expect("storage mutex poisoned");
    application::list_opportunities(&storage, include_archived).map_err(Into::into)
}

#[tauri::command]
fn get_opportunity(
    storage: State<'_, SharedStorage>,
    opportunity_id: String,
) -> Result<OpportunityDetail, CommandError> {
    let storage = storage.lock().expect("storage mutex poisoned");
    application::get_opportunity(&storage, &opportunity_id).map_err(Into::into)
}

#[tauri::command]
fn move_opportunity_stage(
    storage: State<'_, SharedStorage>,
    request: MoveOpportunityStageRequest,
) -> Result<Opportunity, CommandError> {
    let mut storage = storage.lock().expect("storage mutex poisoned");
    application::move_opportunity_stage(&mut storage, request).map_err(Into::into)
}

// Activity commands — logged touches and the unified timeline.

#[tauri::command]
fn log_activity(
    storage: State<'_, SharedStorage>,
    request: LogActivityRequest,
) -> Result<Activity, CommandError> {
    let mut storage = storage.lock().expect("storage mutex poisoned");
    application::log_activity(&mut storage, request).map_err(Into::into)
}

#[tauri::command]
fn update_activity(
    storage: State<'_, SharedStorage>,
    request: UpdateActivityRequest,
) -> Result<Activity, CommandError> {
    let mut storage = storage.lock().expect("storage mutex poisoned");
    application::update_activity(&mut storage, request).map_err(Into::into)
}

#[tauri::command]
fn delete_activity(
    storage: State<'_, SharedStorage>,
    request: DeleteActivityRequest,
) -> Result<(), CommandError> {
    let mut storage = storage.lock().expect("storage mutex poisoned");
    application::delete_activity(&mut storage, request).map_err(Into::into)
}

#[tauri::command]
fn get_timeline(
    storage: State<'_, SharedStorage>,
    parent_type: String,
    parent_id: String,
    include_related: bool,
) -> Result<Vec<Activity>, CommandError> {
    let storage = storage.lock().expect("storage mutex poisoned");
    application::get_timeline(&storage, &parent_type, &parent_id, include_related)
        .map_err(Into::into)
}

// Task commands — follow-ups with due dates, reminders, and priorities.

#[tauri::command]
fn create_task(
    storage: State<'_, SharedStorage>,
    request: CreateTaskRequest,
) -> Result<Task, CommandError> {
    let mut storage = storage.lock().expect("storage mutex poisoned");
    application::create_task(&mut storage, request).map_err(Into::into)
}

#[tauri::command]
fn update_task(
    storage: State<'_, SharedStorage>,
    request: UpdateTaskRequest,
) -> Result<Task, CommandError> {
    let mut storage = storage.lock().expect("storage mutex poisoned");
    application::update_task(&mut storage, request).map_err(Into::into)
}

#[tauri::command]
fn complete_task(
    storage: State<'_, SharedStorage>,
    request: CompleteTaskRequest,
) -> Result<Task, CommandError> {
    let mut storage = storage.lock().expect("storage mutex poisoned");
    application::complete_task(&mut storage, request).map_err(Into::into)
}

#[tauri::command]
fn reopen_task(
    storage: State<'_, SharedStorage>,
    request: TaskActionRequest,
) -> Result<Task, CommandError> {
    let mut storage = storage.lock().expect("storage mutex poisoned");
    application::reopen_task(&mut storage, request).map_err(Into::into)
}

#[tauri::command]
fn drop_task(
    storage: State<'_, SharedStorage>,
    request: TaskActionRequest,
) -> Result<Task, CommandError> {
    let mut storage = storage.lock().expect("storage mutex poisoned");
    application::drop_task(&mut storage, request).map_err(Into::into)
}

#[tauri::command]
fn delete_task(
    storage: State<'_, SharedStorage>,
    request: TaskActionRequest,
) -> Result<(), CommandError> {
    let mut storage = storage.lock().expect("storage mutex poisoned");
    application::delete_task(&mut storage, request).map_err(Into::into)
}

#[tauri::command]
fn list_tasks(
    storage: State<'_, SharedStorage>,
    request: ListTasksRequest,
) -> Result<Vec<Task>, CommandError> {
    let storage = storage.lock().expect("storage mutex poisoned");
    application::list_tasks(&storage, request).map_err(Into::into)
}

// Hand-off commands — quote/job references and the versioned envelope export.

#[tauri::command]
fn link_quote(
    storage: State<'_, SharedStorage>,
    request: LinkQuoteRequest,
) -> Result<Opportunity, CommandError> {
    let mut storage = storage.lock().expect("storage mutex poisoned");
    application::link_quote(&mut storage, request).map_err(Into::into)
}

#[tauri::command]
fn unlink_quote(
    storage: State<'_, SharedStorage>,
    request: UnlinkHandoffRequest,
) -> Result<Opportunity, CommandError> {
    let mut storage = storage.lock().expect("storage mutex poisoned");
    application::unlink_quote(&mut storage, request).map_err(Into::into)
}

#[tauri::command]
fn link_job(
    storage: State<'_, SharedStorage>,
    request: LinkJobRequest,
) -> Result<Opportunity, CommandError> {
    let mut storage = storage.lock().expect("storage mutex poisoned");
    application::link_job(&mut storage, request).map_err(Into::into)
}

#[tauri::command]
fn unlink_job(
    storage: State<'_, SharedStorage>,
    request: UnlinkHandoffRequest,
) -> Result<Opportunity, CommandError> {
    let mut storage = storage.lock().expect("storage mutex poisoned");
    application::unlink_job(&mut storage, request).map_err(Into::into)
}

#[tauri::command]
fn export_handoff_envelope(
    storage: State<'_, SharedStorage>,
    opportunity_id: String,
    destination_path: String,
    overwrite: bool,
) -> Result<EnvelopeExportReport, CommandError> {
    let mut storage = storage.lock().expect("storage mutex poisoned");
    application::export_handoff_envelope(
        &mut storage,
        &opportunity_id,
        &destination_path,
        overwrite,
    )
    .map_err(Into::into)
}

// Needs-attention commands — deterministic flags and their thresholds.

#[tauri::command]
fn get_attention_flags(
    storage: State<'_, SharedStorage>,
    reference_time: Option<String>,
) -> Result<Vec<AttentionFlag>, CommandError> {
    let storage = storage.lock().expect("storage mutex poisoned");
    application::get_attention_flags(&storage, reference_time).map_err(Into::into)
}

#[tauri::command]
fn get_attention_thresholds(storage: State<'_, SharedStorage>) -> Result<Thresholds, CommandError> {
    let storage = storage.lock().expect("storage mutex poisoned");
    application::get_attention_thresholds(&storage).map_err(Into::into)
}

#[tauri::command]
fn set_attention_thresholds(
    storage: State<'_, SharedStorage>,
    request: SetAttentionThresholdsRequest,
) -> Result<Thresholds, CommandError> {
    let mut storage = storage.lock().expect("storage mutex poisoned");
    application::set_attention_thresholds(&mut storage, request).map_err(Into::into)
}

// Database maintenance commands — backup/restore snapshots and storage info.

#[tauri::command]
fn backup_database(
    storage: State<'_, SharedStorage>,
    destination_path: String,
    overwrite: bool,
) -> Result<DatabaseInfo, CommandError> {
    let mut storage = storage.lock().expect("storage mutex poisoned");
    application::backup_database(&mut storage, &destination_path, overwrite).map_err(Into::into)
}

#[tauri::command]
fn restore_database(
    storage: State<'_, SharedStorage>,
    backup_path: String,
) -> Result<RestoreReport, CommandError> {
    let mut storage = storage.lock().expect("storage mutex poisoned");
    application::restore_database(&mut storage, &backup_path).map_err(Into::into)
}

#[tauri::command]
fn get_database_info(storage: State<'_, SharedStorage>) -> Result<DatabaseInfo, CommandError> {
    let storage = storage.lock().expect("storage mutex poisoned");
    application::get_database_info(&storage).map_err(Into::into)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            // Open and migrate the database in the Tauri app data dir; commands
            // reach it through managed state (Connection is Send, not Sync).
            let app_data = app.path().app_data_dir()?;
            let storage = Storage::open_in_app_data(app_data)?;
            app.manage(Mutex::new(storage));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            health,
            create_company,
            update_company,
            archive_company,
            unarchive_company,
            list_companies,
            get_company,
            create_contact,
            update_contact,
            archive_contact,
            unarchive_contact,
            list_contacts,
            get_contact,
            list_stages,
            update_stage,
            list_lost_reasons,
            create_opportunity,
            update_opportunity,
            archive_opportunity,
            unarchive_opportunity,
            list_opportunities,
            get_opportunity,
            move_opportunity_stage,
            log_activity,
            update_activity,
            delete_activity,
            get_timeline,
            create_task,
            update_task,
            complete_task,
            reopen_task,
            drop_task,
            delete_task,
            list_tasks,
            link_quote,
            unlink_quote,
            link_job,
            unlink_job,
            export_handoff_envelope,
            get_attention_flags,
            get_attention_thresholds,
            set_attention_thresholds,
            backup_database,
            restore_database,
            get_database_info
        ])
        .run(tauri::generate_context!())
        .expect("error while running ContractorCRM");
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{health_report, ApplicationError, CommandError};

    #[test]
    fn health_report_identifies_the_app_and_is_ok() {
        let report = health_report();
        assert_eq!(report.app, "ContractorCRM");
        assert_eq!(report.status, "ok");
        assert_eq!(report.version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn health_report_serializes_to_the_camel_case_wire_shape() {
        assert_eq!(
            serde_json::to_value(health_report()).expect("serialize health report"),
            json!({
                "app": "ContractorCRM",
                "version": env!("CARGO_PKG_VERSION"),
                "status": "ok"
            })
        );
    }

    #[test]
    fn validation_command_error_includes_code_and_field_path() {
        let command_error = CommandError::from(ApplicationError::ValidationFailed {
            code: "company_has_active_contacts",
            field: "companyId".into(),
            message: "cannot archive company with active contacts".into(),
        });

        assert_eq!(
            serde_json::to_value(command_error).expect("serialize command error"),
            json!({
                "kind": "validation_failed",
                "message": "cannot archive company with active contacts",
                "code": "company_has_active_contacts",
                "field": "companyId"
            })
        );
    }

    #[test]
    fn version_conflict_command_error_carries_current_version() {
        let command_error = CommandError::from(ApplicationError::VersionConflict {
            resource: "contact",
            id: "abc".into(),
            expected: 1,
            current: 3,
        });

        assert_eq!(
            serde_json::to_value(command_error).expect("serialize command error"),
            json!({
                "kind": "version_conflict",
                "message": "contact abc changed: expected version 1, current version 3",
                "resource": "contact",
                "recordId": "abc",
                "expectedVersion": 1,
                "currentVersion": 3
            })
        );
    }
}
