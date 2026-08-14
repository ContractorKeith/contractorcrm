pub mod application;
pub mod domain;
pub mod error;
pub mod storage;

use std::sync::Mutex;

use application::{
    ArchiveRequest, CreateCompanyRequest, CreateContactRequest, UpdateCompanyRequest,
    UpdateContactRequest,
};
use domain::{Company, Contact};
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
) -> Result<Vec<Contact>, CommandError> {
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
            get_contact
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
