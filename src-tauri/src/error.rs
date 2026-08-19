use serde::Serialize;
use thiserror::Error;

/// Errors surfaced by the storage layer (open/migrate); the application layer
/// wraps these into `ApplicationError`.
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("stored data is invalid: {0}")]
    InvalidStoredData(String),

    #[error("backup failed: {0}")]
    BackupFailed(String),

    #[error("restore rejected: {0}")]
    RestoreInvalid(String),

    #[error("local database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("local storage error: {0}")]
    Io(#[from] std::io::Error),
}

/// Typed application error with the stable kinds from docs/LOCAL_API.md.
/// `field` is a String so channel paths like `channels[1].preferred` work.
#[derive(Debug, Error)]
pub enum ApplicationError {
    #[error("{field}: {message}")]
    InvalidInput { field: String, message: String },

    #[error("{resource} {id} was not found")]
    NotFound { resource: &'static str, id: String },

    #[error("{message}")]
    ValidationFailed {
        code: &'static str,
        field: String,
        message: String,
    },

    #[error("moving opportunity {id} to the lost stage requires a lost reason")]
    MissingLostReason { id: String },

    #[error("{resource} {id} changed: expected version {expected}, current version {current}")]
    VersionConflict {
        resource: &'static str,
        id: String,
        expected: i64,
        current: i64,
    },

    #[error("stored data is invalid: {0}")]
    InvalidStoredData(String),

    #[error("backup failed: {0}")]
    BackupFailed(String),

    #[error("restore rejected: {0}")]
    RestoreInvalid(String),

    /// The configured AI provider (or the credential store it needs) could not
    /// be reached. `reason` is user-facing text and never carries secrets.
    #[error("{reason}")]
    ProviderUnavailable { reason: String },

    /// The draft is gone — expired, already applied, or never existed. All
    /// three look the same to the caller: ask the assistant again.
    #[error("that draft is no longer available; ask the assistant again")]
    ProposalExpired { proposal_id: String },

    /// The caller is connected read-only (the agent interface's read mode);
    /// defined here, returned by the MCP adapter.
    #[error("this connection is read-only: {command} is not available")]
    ReadOnly { command: String },

    #[error("local database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("local storage error: {0}")]
    Io(#[from] std::io::Error),
}

impl ApplicationError {
    /// Stable machine-readable error kind for the UI and agent API.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::InvalidInput { .. } => "invalid_input",
            Self::NotFound { .. } => "not_found",
            Self::ValidationFailed { .. } => "validation_failed",
            Self::MissingLostReason { .. } => "missing_lost_reason",
            Self::VersionConflict { .. } => "version_conflict",
            Self::InvalidStoredData(_) => "invalid_stored_data",
            Self::BackupFailed(_) => "backup_failed",
            Self::RestoreInvalid(_) => "restore_invalid",
            Self::ProviderUnavailable { .. } => "provider_unavailable",
            Self::ProposalExpired { .. } => "proposal_expired",
            Self::ReadOnly { .. } => "read_only",
            Self::Database(_) => "storage_unavailable",
            Self::Io(_) => "io",
        }
    }
}

/// Wire shape for application errors — stable kind plus per-kind details.
/// Shared by the Tauri command layer and the MCP adapter so both surfaces
/// report exactly the same JSON for the same failure.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    pub kind: &'static str,
    pub message: String,
    #[serde(flatten)]
    pub details: Box<CommandErrorDetails>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all_fields = "camelCase", untagged)]
pub enum CommandErrorDetails {
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
    /// Why the AI provider could not be used — safe text only, never a key.
    Provider {
        reason: String,
    },
    /// Which draft went away, so the caller can drop it from its state.
    Proposal {
        proposal_id: String,
    },
    /// Which command a read-only connection refused.
    ReadOnly {
        command: String,
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
            ApplicationError::ProviderUnavailable { reason } => CommandErrorDetails::Provider {
                reason: reason.clone(),
            },
            ApplicationError::ProposalExpired { proposal_id } => CommandErrorDetails::Proposal {
                proposal_id: proposal_id.clone(),
            },
            ApplicationError::ReadOnly { command } => CommandErrorDetails::ReadOnly {
                command: command.clone(),
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

impl From<StorageError> for ApplicationError {
    fn from(error: StorageError) -> Self {
        match error {
            StorageError::InvalidStoredData(message) => Self::InvalidStoredData(message),
            StorageError::BackupFailed(message) => Self::BackupFailed(message),
            StorageError::RestoreInvalid(message) => Self::RestoreInvalid(message),
            StorageError::Database(inner) => Self::Database(inner),
            StorageError::Io(inner) => Self::Io(inner),
        }
    }
}
