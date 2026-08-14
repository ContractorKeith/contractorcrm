use std::path::{Path, PathBuf};

use chrono::{SecondsFormat, Utc};
use rusqlite::{params, Connection, OpenFlags, TransactionBehavior};
use uuid::Uuid;

use crate::error::StorageError;

/// Default database file name inside the application data directory.
pub const DATABASE_FILE_NAME: &str = "contractorcrm.sqlite3";

/// One forward-only schema migration; SQL runs inside a single transaction.
struct Migration {
    version: i64,
    sql: &'static str,
}

/// v1 core tables per docs/DATA_MODEL.md — companies, contacts, contact
/// channels, command log, and app settings. Timestamps are UTC ISO-8601 TEXT,
/// ids are UUIDv7-style TEXT, record versions start at 1.
const MIGRATION_001: &str = "\
CREATE TABLE companies (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    phone TEXT,
    email TEXT,
    website TEXT,
    address_line1 TEXT,
    address_line2 TEXT,
    city TEXT,
    state TEXT,
    postal_code TEXT,
    service_area TEXT,
    license_notes TEXT,
    notes TEXT,
    archived_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1 CHECK (version > 0)
);
CREATE INDEX companies_active_name
    ON companies(name) WHERE archived_at IS NULL;

CREATE TABLE contacts (
    id TEXT PRIMARY KEY,
    company_id TEXT,
    first_name TEXT,
    last_name TEXT,
    display_name TEXT NOT NULL,
    role TEXT,
    kind TEXT NOT NULL,
    preferred_contact_method TEXT,
    address_line1 TEXT,
    address_line2 TEXT,
    city TEXT,
    state TEXT,
    postal_code TEXT,
    property_type TEXT,
    notes TEXT,
    favorite INTEGER NOT NULL DEFAULT 0 CHECK (favorite IN (0, 1)),
    archived_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1 CHECK (version > 0),
    FOREIGN KEY (company_id) REFERENCES companies(id) ON DELETE RESTRICT
);
CREATE INDEX contacts_company ON contacts(company_id);
CREATE INDEX contacts_active_display_name
    ON contacts(display_name) WHERE archived_at IS NULL;

CREATE TABLE contact_channels (
    id TEXT PRIMARY KEY,
    contact_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    label TEXT,
    value TEXT NOT NULL,
    preferred INTEGER NOT NULL DEFAULT 0 CHECK (preferred IN (0, 1)),
    sort_key INTEGER NOT NULL DEFAULT 0 CHECK (sort_key >= 0),
    FOREIGN KEY (contact_id) REFERENCES contacts(id) ON DELETE CASCADE
);
CREATE INDEX contact_channels_contact ON contact_channels(contact_id, sort_key);

CREATE TABLE command_log (
    id TEXT PRIMARY KEY,
    command_id TEXT NOT NULL,
    actor TEXT NOT NULL CHECK (actor IN ('user', 'agent', 'import')),
    entity_type TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    summary TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX command_log_entity ON command_log(entity_type, entity_id, created_at);

CREATE TABLE app_settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";

/// Ordered, forward-only migration list; append new versions, never edit old ones.
const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    sql: MIGRATION_001,
}];

/// Owns the SQLite connection; the repository layer builds on it. The UI and
/// agents never touch SQLite directly — every write goes through this seam.
pub struct Storage {
    database_path: PathBuf,
    connection: Connection,
}

impl Storage {
    /// Open (creating if needed) the database at an explicit path — the test
    /// seam. Production callers use `open_in_app_data`.
    pub fn open(database_path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let database_path = database_path.as_ref().to_path_buf();
        let database_existed = database_path.exists();
        if let Some(parent) = database_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let connection = Connection::open(&database_path)?;
        // WAL for concurrent read friendliness; foreign keys are per-connection.
        connection.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")?;

        let mut storage = Self {
            database_path,
            connection,
        };
        storage.migrate(database_existed)?;
        Ok(storage)
    }

    /// Open the database in the given application data directory (Tauri app
    /// data dir in production) under the default file name.
    pub fn open_in_app_data(app_data_dir: impl AsRef<Path>) -> Result<Self, StorageError> {
        Self::open(app_data_dir.as_ref().join(DATABASE_FILE_NAME))
    }

    /// Borrow the underlying connection for the repository layer and tests.
    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    /// Mutable borrow so the application layer can open transactions.
    pub fn connection_mut(&mut self) -> &mut Connection {
        &mut self.connection
    }

    /// Where the live database file lives on disk.
    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    /// Write a consistent, compact snapshot of the open database to
    /// `destination` via `VACUUM INTO`. Refuses to overwrite an existing file
    /// unless `overwrite` is set; creates missing parent directories.
    pub fn backup_to(
        &self,
        destination: impl AsRef<Path>,
        overwrite: bool,
    ) -> Result<(), StorageError> {
        let destination = destination.as_ref();
        if destination.exists() {
            if !overwrite {
                return Err(StorageError::BackupFailed(format!(
                    "{} already exists; enable overwrite to replace it",
                    destination.display()
                )));
            }
            // VACUUM INTO refuses existing files, so clear the old one first.
            std::fs::remove_file(destination)?;
        }
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let destination_text = destination.to_str().ok_or_else(|| {
            StorageError::BackupFailed("destination path is not valid UTF-8".into())
        })?;
        self.connection
            .execute("VACUUM INTO ?1", [destination_text])
            .map_err(|error| StorageError::BackupFailed(error.to_string()))?;
        Ok(())
    }

    /// Verify a backup file without touching the live database: it must open
    /// read-only, pass PRAGMA integrity_check, carry a schema_migrations
    /// table, and not be newer than this build's latest known migration.
    pub fn verify_backup_file(backup_path: impl AsRef<Path>) -> Result<(), StorageError> {
        let backup_path = backup_path.as_ref();
        if !backup_path.is_file() {
            return Err(StorageError::RestoreInvalid(format!(
                "{} is not a file",
                backup_path.display()
            )));
        }
        let connection = Connection::open_with_flags(backup_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| {
                StorageError::RestoreInvalid(format!("cannot open backup: {error}"))
            })?;
        // SQLite opens lazily, so a garbage file fails here, not at open.
        let integrity: String = connection
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .map_err(|error| {
                StorageError::RestoreInvalid(format!("not a readable database: {error}"))
            })?;
        if integrity != "ok" {
            return Err(StorageError::RestoreInvalid(format!(
                "integrity check failed: {integrity}"
            )));
        }
        let has_migrations_table: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master
             WHERE type = 'table' AND name = 'schema_migrations')",
            [],
            |row| row.get(0),
        )?;
        if !has_migrations_table {
            return Err(StorageError::RestoreInvalid(
                "no schema_migrations table; not a ContractorCRM backup".into(),
            ));
        }
        let backup_version: i64 = connection.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )?;
        let supported = latest_migration_version();
        if backup_version > supported {
            return Err(StorageError::RestoreInvalid(format!(
                "backup schema version {backup_version} is newer than this app \
                 supports ({supported}); update the app first"
            )));
        }
        Ok(())
    }

    /// Replace the live database with a verified backup and return the path of
    /// the timestamped pre-restore safety copy. Verification happens before the
    /// live database is touched; the file swap uses a staged copy plus rename
    /// (atomic on the same filesystem); reopening runs migrations forward for
    /// older backups.
    pub fn restore_from(&mut self, backup_path: impl AsRef<Path>) -> Result<PathBuf, StorageError> {
        let backup_path = backup_path.as_ref();
        Self::verify_backup_file(backup_path)?;

        let file_name = self
            .database_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                StorageError::InvalidStoredData("database path has no file name".into())
            })?
            .to_string();

        // Consistent timestamped safety copy of the live database, next to it.
        let stamp = Utc::now().format("%Y%m%dT%H%M%S%3fZ");
        let safety_path = self
            .database_path
            .with_file_name(format!("{file_name}.pre-restore-{stamp}.bak"));
        self.connection.backup("main", &safety_path, None)?;

        // Close the live connection (swap in a throwaway in-memory one) so the
        // file can be replaced; drop the WAL/SHM sidecars with it.
        drop(std::mem::replace(
            &mut self.connection,
            Connection::open_in_memory()?,
        ));
        for suffix in ["-wal", "-shm"] {
            let mut sidecar = self.database_path.clone().into_os_string();
            sidecar.push(suffix);
            let _ = std::fs::remove_file(sidecar); // best effort — may not exist
        }

        // Stage next to the live file, then rename over it.
        let staged_path = self
            .database_path
            .with_file_name(format!("{file_name}.restore-staging"));
        std::fs::copy(backup_path, &staged_path)?;
        std::fs::rename(&staged_path, &self.database_path)?;

        // Reopen and migrate forward; replaces the throwaway connection.
        let database_path = self.database_path.clone();
        *self = Self::open(database_path)?;
        Ok(safety_path)
    }

    /// Apply any pending migrations; already-applied versions are skipped, so
    /// re-running on an existing database is a no-op.
    fn migrate(&mut self, database_existed: bool) -> Result<(), StorageError> {
        self.connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at TEXT NOT NULL
             );",
        )?;

        for migration in MIGRATIONS {
            if migration_applied(&self.connection, migration.version)? {
                continue;
            }
            // Safety net: keep a pre-migration copy of any pre-existing database.
            if database_existed {
                self.back_up_before_migration(migration.version)?;
            }
            let transaction = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(migration.sql)?;
            transaction.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                params![migration.version, now_utc()],
            )?;
            transaction.commit()?;
        }
        Ok(())
    }

    /// Copy the database file aside before a migration touches an existing one.
    fn back_up_before_migration(&self, target_version: i64) -> Result<(), StorageError> {
        let file_name = self
            .database_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                StorageError::InvalidStoredData("database path has no file name".into())
            })?;
        let backup_path = self
            .database_path
            .with_file_name(format!("{file_name}.pre-migration-v{target_version}.bak"));
        if !backup_path.exists() {
            self.connection.backup("main", backup_path, None)?;
        }
        Ok(())
    }
}

/// Latest schema version this build knows how to open.
pub fn latest_migration_version() -> i64 {
    MIGRATIONS
        .last()
        .map(|migration| migration.version)
        .unwrap_or(0)
}

/// True when a migration version is already recorded in schema_migrations.
fn migration_applied(connection: &Connection, version: i64) -> Result<bool, StorageError> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version = ?1)",
        [version],
        |row| row.get(0),
    )?)
}

/// New UUIDv7-style record id — time-ordered, safe for offline generation.
pub fn new_id() -> String {
    Uuid::now_v7().to_string()
}

/// Current UTC timestamp as ISO-8601 with millisecond precision.
pub fn now_utc() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}
