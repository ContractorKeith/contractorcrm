use thiserror::Error;

/// Errors surfaced by the storage layer; the repository layer will extend this
/// with domain variants (not-found, version conflict) in later issues.
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("stored data is invalid: {0}")]
    InvalidStoredData(String),

    #[error("local database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("local storage error: {0}")]
    Io(#[from] std::io::Error),
}
