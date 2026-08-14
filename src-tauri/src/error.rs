use thiserror::Error;

/// Errors surfaced by the storage layer (open/migrate); the application layer
/// wraps these into `ApplicationError`.
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("stored data is invalid: {0}")]
    InvalidStoredData(String),

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

    #[error("{resource} {id} changed: expected version {expected}, current version {current}")]
    VersionConflict {
        resource: &'static str,
        id: String,
        expected: i64,
        current: i64,
    },

    #[error("stored data is invalid: {0}")]
    InvalidStoredData(String),

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
            Self::VersionConflict { .. } => "version_conflict",
            Self::InvalidStoredData(_) => "invalid_stored_data",
            Self::Database(_) | Self::Io(_) => "storage_unavailable",
        }
    }
}

impl From<StorageError> for ApplicationError {
    fn from(error: StorageError) -> Self {
        match error {
            StorageError::InvalidStoredData(message) => Self::InvalidStoredData(message),
            StorageError::Database(inner) => Self::Database(inner),
            StorageError::Io(inner) => Self::Io(inner),
        }
    }
}
