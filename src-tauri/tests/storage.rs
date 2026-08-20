use contractorcrm_lib::error::{ApplicationError, StorageError};
use contractorcrm_lib::storage::{new_id, now_utc, Migration, Storage};
use rusqlite::params;
use std::io::{Seek, SeekFrom, Write};

/// Test-only migration list: the first statement succeeds, the second is not
/// SQL. Runs on top of an already-migrated database through
/// `Storage::open_with_migrations`.
const FAILING_MIGRATIONS: &[Migration] = &[Migration {
    version: 9001,
    sql: "CREATE TABLE half_applied_v9001 (id TEXT PRIMARY KEY);\n\
          ALTER TABLE contacts ADD COLUMN half_applied_column TEXT;\n\
          THIS IS NOT SQL;",
}];

/// `Storage` is not `Debug`, so unwrap the failure side by hand.
fn expect_open_failure(result: Result<Storage, StorageError>, context: &str) -> StorageError {
    match result {
        Ok(_) => panic!("{context}"),
        Err(error) => error,
    }
}

/// Schema version recorded in a database file, read without migrating it.
fn backup_schema_version(path: &std::path::Path) -> i64 {
    let connection =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("open backup read-only");
    connection
        .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .expect("read backup version")
}

/// Table names the migrations must create, plus the migration ledger itself.
const EXPECTED_TABLES: &[&str] = &[
    "activities",
    "app_settings",
    "attachments",
    "command_log",
    "companies",
    "contact_channels",
    "contacts",
    "custom_field_defs",
    "custom_field_options",
    "custom_field_values",
    "lost_reasons",
    "opportunities",
    "pipelines",
    "record_tags",
    "saved_views",
    "schema_migrations",
    "search_index",
    "stage_history",
    "stages",
    "tags",
    "tasks",
];

fn table_names(storage: &Storage) -> Vec<String> {
    let mut statement = storage
        .connection()
        .prepare(
            "SELECT name FROM sqlite_master
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE 'search_index_%'
             ORDER BY name",
        )
        .expect("prepare table listing");
    statement
        .query_map([], |row| row.get(0))
        .expect("query table names")
        .collect::<rusqlite::Result<Vec<String>>>()
        .expect("collect table names")
}

fn applied_versions(storage: &Storage) -> Vec<i64> {
    let mut statement = storage
        .connection()
        .prepare("SELECT version FROM schema_migrations ORDER BY version")
        .expect("prepare version listing");
    statement
        .query_map([], |row| row.get(0))
        .expect("query versions")
        .collect::<rusqlite::Result<Vec<i64>>>()
        .expect("collect versions")
}

#[test]
fn fresh_database_gets_all_tables_and_pragmas() {
    let temp = tempfile::tempdir().expect("create temporary app data");
    let storage = Storage::open_in_app_data(temp.path()).expect("open storage");

    assert_eq!(table_names(&storage), EXPECTED_TABLES);

    let journal_mode: String = storage
        .connection()
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .expect("read journal_mode");
    assert_eq!(journal_mode.to_lowercase(), "wal");

    let foreign_keys: i64 = storage
        .connection()
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .expect("read foreign_keys");
    assert_eq!(foreign_keys, 1);
}

#[test]
fn schema_migrations_records_versions_and_rerunning_is_a_no_op() {
    let temp = tempfile::tempdir().expect("create temporary app data");
    let database_path = temp.path().join("contractorcrm.sqlite3");

    let storage = Storage::open(&database_path).expect("first open");
    assert_eq!(
        applied_versions(&storage),
        vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]
    );
    drop(storage);

    // Reopening re-runs the migration framework; applied versions are skipped.
    let reopened = Storage::open(&database_path).expect("second open");
    assert_eq!(
        applied_versions(&reopened),
        vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]
    );
    assert_eq!(table_names(&reopened), EXPECTED_TABLES);
}

#[test]
fn existing_data_survives_a_migration_rerun() {
    let temp = tempfile::tempdir().expect("create temporary app data");
    let database_path = temp.path().join("contractorcrm.sqlite3");

    let storage = Storage::open(&database_path).expect("first open");
    let company_id = new_id();
    let now = now_utc();
    storage
        .connection()
        .execute(
            "INSERT INTO companies (id, name, kind, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![company_id, "Ridgeline Fence Co", "sub", now, now],
        )
        .expect("insert company");
    drop(storage);

    let reopened = Storage::open(&database_path).expect("reopen with data");
    let (name, version): (String, i64) = reopened
        .connection()
        .query_row(
            "SELECT name, version FROM companies WHERE id = ?1",
            [&company_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("company survives reopen");
    assert_eq!(name, "Ridgeline Fence Co");
    assert_eq!(version, 1); // DEFAULT 1 applied

    let command_log_actor_check: i64 = reopened
        .connection()
        .execute(
            "INSERT INTO command_log (id, command_id, actor, entity_type, entity_id, summary, created_at)
             VALUES (?1, ?2, 'user', 'company', ?3, 'created company', ?4)",
            params![new_id(), new_id(), company_id, now_utc()],
        )
        .expect("valid actor accepted") as i64;
    assert_eq!(command_log_actor_check, 1);

    // Foreign keys are enforced on the open connection.
    let orphan_channel = reopened.connection().execute(
        "INSERT INTO contact_channels (id, contact_id, kind, value)
         VALUES (?1, 'missing-contact', 'phone', '555-0100')",
        [new_id()],
    );
    assert!(orphan_channel.is_err(), "orphan channel must be rejected");
}

#[test]
fn populated_v7_database_upgrades_with_backup_and_preserves_core_projections() {
    let temp = tempfile::tempdir().expect("tempdir");
    let database_path = temp.path().join("contractorcrm.sqlite3");
    let storage = Storage::open(&database_path).expect("create current database");
    let now = now_utc();
    storage.connection().execute(
        "INSERT INTO companies (id,name,kind,created_at,updated_at,version) VALUES ('upgrade-company','Upgrade Company','client',?1,?1,1)",
        [&now],
    ).expect("seed company");
    storage.connection().execute(
        "INSERT INTO search_index (entity_type,entity_id,title,content) VALUES ('company','upgrade-company','Upgrade Company','Upgrade Company durable')",
        [],
    ).expect("seed search projection");
    let v1_definition = r#"{"schemaVersion":1,"filter":{"includeArchived":false},"sort":{"field":"name","direction":"ascending"}}"#;
    storage.connection().execute(
        "INSERT INTO saved_views (id,name,entity_type,definition_json,sort_key,created_at,updated_at,version) VALUES ('upgrade-view','Upgrade view','company',?1,0,?2,?2,1)",
        params![v1_definition, now],
    ).expect("seed v1 saved view");
    storage
        .connection()
        .execute_batch(
            "DROP TRIGGER contacts_metadata_delete;
         DROP TRIGGER companies_metadata_delete;
         DROP TRIGGER opportunities_metadata_delete;
         DROP TABLE custom_field_values;
         DROP TABLE custom_field_options;
         DROP TABLE custom_field_defs;
         DROP TABLE record_tags;
         DROP TABLE tags;
         DROP INDEX contacts_external_id_unique;
         ALTER TABLE contacts DROP COLUMN external_id;
         DELETE FROM schema_migrations WHERE version=8;
         DELETE FROM schema_migrations WHERE version=9;",
        )
        .expect("return fixture to populated v7");
    drop(storage);

    let upgraded = Storage::open(&database_path).expect("upgrade v7 to v8");
    assert_eq!(
        applied_versions(&upgraded),
        vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]
    );
    assert_eq!(
        upgraded
            .connection()
            .query_row::<String, _, _>(
                "SELECT name FROM companies WHERE id='upgrade-company'",
                [],
                |row| row.get(0),
            )
            .unwrap(),
        "Upgrade Company"
    );
    assert_eq!(
        upgraded
            .connection()
            .query_row::<String, _, _>(
                "SELECT definition_json FROM saved_views WHERE id='upgrade-view'",
                [],
                |row| row.get(0),
            )
            .unwrap(),
        v1_definition
    );
    let search_count: i64 = upgraded.connection().query_row(
        "SELECT COUNT(*) FROM search_index WHERE entity_type='company' AND entity_id='upgrade-company' AND search_index MATCH 'durable'",
        [],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(search_count, 1);
    assert!(database_path
        .with_file_name("contractorcrm.sqlite3.pre-migration-v8.bak")
        .is_file());
    drop(upgraded);

    let reopened = Storage::open(&database_path).expect("idempotent reopen");
    assert_eq!(
        applied_versions(&reopened),
        vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]
    );
}

#[test]
fn populated_v8_database_gains_contact_external_ids() {
    let temp = tempfile::tempdir().expect("tempdir");
    let database_path = temp.path().join("contractorcrm.sqlite3");
    let storage = Storage::open(&database_path).expect("create current database");
    let now = now_utc();
    storage
        .connection()
        .execute(
            "INSERT INTO contacts (id,display_name,kind,created_at,updated_at,version)
             VALUES ('legacy-contact','Legacy Contact','client',?1,?1,1)",
            [&now],
        )
        .expect("seed contact");
    storage
        .connection()
        .execute_batch(
            "DROP INDEX contacts_external_id_unique;
             ALTER TABLE contacts DROP COLUMN external_id;
             DELETE FROM schema_migrations WHERE version=9;",
        )
        .expect("return fixture to populated v8");
    drop(storage);

    let upgraded = Storage::open(&database_path).expect("upgrade v8 to v9");
    assert_eq!(
        applied_versions(&upgraded),
        vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]
    );
    let external_id: Option<String> = upgraded
        .connection()
        .query_row(
            "SELECT external_id FROM contacts WHERE id='legacy-contact'",
            [],
            |row| row.get(0),
        )
        .expect("existing contact keeps its row and gains a null external id");
    assert_eq!(external_id, None);

    // The partial unique index allows many NULLs but one row per external id.
    upgraded
        .connection()
        .execute(
            "UPDATE contacts SET external_id='crm-1' WHERE id='legacy-contact'",
            [],
        )
        .expect("set external id");
    let now = now_utc();
    upgraded
        .connection()
        .execute(
            "INSERT INTO contacts (id,display_name,kind,external_id,created_at,updated_at,version)
             VALUES ('other-contact','Other Contact','client',NULL,?1,?1,1)",
            [&now],
        )
        .expect("null external ids are unconstrained");
    let duplicate = upgraded.connection().execute(
        "UPDATE contacts SET external_id='crm-1' WHERE id='other-contact'",
        [],
    );
    assert!(duplicate.is_err(), "duplicate external id must be rejected");
    assert!(database_path
        .with_file_name("contractorcrm.sqlite3.pre-migration-v9.bak")
        .is_file());
}

#[test]
fn a_failing_migration_leaves_no_schema_or_ledger_trace() {
    let temp = tempfile::tempdir().expect("tempdir");
    let database_path = temp.path().join("contractorcrm.sqlite3");
    let storage = Storage::open(&database_path).expect("create current database");
    let now = now_utc();
    storage
        .connection()
        .execute(
            "INSERT INTO contacts (id,display_name,kind,created_at,updated_at,version)
             VALUES ('pre-migration-contact','Pre Migration','client',?1,?1,1)",
            [&now],
        )
        .expect("seed contact");
    drop(storage);

    let failure = expect_open_failure(
        Storage::open_with_migrations(&database_path, FAILING_MIGRATIONS),
        "a broken migration must not open",
    );
    assert!(
        matches!(failure, StorageError::Database(_)),
        "unexpected error: {failure}"
    );
    // The safety net still ran: an existing database gets a copy before the
    // migration touches it, whether or not the migration then succeeds.
    assert!(database_path
        .with_file_name("contractorcrm.sqlite3.pre-migration-v9001.bak")
        .is_file());

    // Reopening the normal way must find the old version, untouched.
    let reopened = Storage::open(&database_path).expect("reopen after failed migration");
    assert_eq!(
        applied_versions(&reopened),
        vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
        "the failed version must not be recorded"
    );
    assert_eq!(table_names(&reopened), EXPECTED_TABLES);
    let half_applied: i64 = reopened
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'half_applied_v9001'",
            [],
            |row| row.get(0),
        )
        .expect("look for the rolled-back table");
    assert_eq!(half_applied, 0, "the failed migration's table survived");
    let column_added: i64 = reopened
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('contacts')
             WHERE name = 'half_applied_column'",
            [],
            |row| row.get(0),
        )
        .expect("look for the rolled-back column");
    assert_eq!(column_added, 0, "the failed migration's column survived");
    let integrity: String = reopened
        .connection()
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .expect("integrity check");
    assert_eq!(integrity, "ok");
    assert_eq!(
        reopened
            .connection()
            .query_row::<String, _, _>(
                "SELECT display_name FROM contacts WHERE id='pre-migration-contact'",
                [],
                |row| row.get(0),
            )
            .expect("data survives a failed migration"),
        "Pre Migration"
    );
}

#[test]
fn restoring_a_pre_migration_backup_returns_to_the_old_version_and_re_migrates() {
    let temp = tempfile::tempdir().expect("tempdir");
    let database_path = temp.path().join("contractorcrm.sqlite3");
    let storage = Storage::open(&database_path).expect("create current database");
    let now = now_utc();
    storage
        .connection()
        .execute(
            "INSERT INTO contacts (id,display_name,kind,created_at,updated_at,version)
             VALUES ('v9-contact','V9 Contact','client',?1,?1,1)",
            [&now],
        )
        .expect("seed contact");
    // Return the fixture to a populated v9 database (pre-attachments).
    storage
        .connection()
        .execute_batch(
            "DROP TRIGGER attachments_owner_insert;
             DROP TRIGGER attachments_owner_update;
             DROP TRIGGER contacts_attachments_delete;
             DROP TRIGGER opportunities_attachments_delete;
             DROP TABLE attachments;
             DELETE FROM schema_migrations WHERE version=10;",
        )
        .expect("downgrade fixture to v9");
    drop(storage);

    // Upgrading writes the pre-migration copy of the v9 database.
    let mut upgraded = Storage::open(&database_path).expect("upgrade v9 to v10");
    assert_eq!(
        applied_versions(&upgraded),
        vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
    );
    let backup_path = database_path.with_file_name("contractorcrm.sqlite3.pre-migration-v10.bak");
    assert!(backup_path.is_file());
    assert_eq!(backup_schema_version(&backup_path), 9, "backup is at v9");
    Storage::verify_backup_file(&backup_path).expect("pre-migration backup verifies");

    // Something written after the upgrade must not survive the restore.
    let now = now_utc();
    upgraded
        .connection()
        .execute(
            "INSERT INTO contacts (id,display_name,kind,created_at,updated_at,version)
             VALUES ('post-upgrade-contact','Post Upgrade','client',?1,?1,1)",
            [&now],
        )
        .expect("seed post-upgrade contact");

    let safety_copy = upgraded
        .restore_from(&backup_path)
        .expect("restore the pre-migration backup");
    assert!(safety_copy.is_file(), "pre-restore safety copy is kept");
    assert_eq!(
        backup_schema_version(&safety_copy),
        10,
        "the safety copy holds the state we restored away from"
    );

    // Restoring reopens through the normal path, so v10 is re-applied cleanly.
    assert_eq!(
        applied_versions(&upgraded),
        vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
    );
    assert_eq!(table_names(&upgraded), EXPECTED_TABLES);
    let survivors: Vec<String> = {
        let mut statement = upgraded
            .connection()
            .prepare("SELECT id FROM contacts ORDER BY id")
            .expect("prepare survivor listing");
        statement
            .query_map([], |row| row.get(0))
            .expect("query survivors")
            .collect::<rusqlite::Result<Vec<String>>>()
            .expect("collect survivors")
    };
    assert_eq!(survivors, vec!["v9-contact".to_string()]);
}

#[test]
fn a_damaged_database_file_opens_with_actionable_guidance() {
    let temp = tempfile::tempdir().expect("tempdir");
    let database_path = temp.path().join("contractorcrm.sqlite3");
    drop(Storage::open(&database_path).expect("create database"));

    // Scribble over the SQLite file header — the classic damaged-file shape.
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(&database_path)
        .expect("open database file");
    file.seek(SeekFrom::Start(0)).expect("seek");
    file.write_all(&[0x7f; 64]).expect("scribble header");
    file.flush().expect("flush");
    drop(file);

    let failure = expect_open_failure(
        Storage::open(&database_path),
        "a damaged file must not open",
    );
    let message = failure.to_string();
    assert!(
        matches!(failure, StorageError::InvalidStoredData(_)),
        "unexpected error: {message}"
    );
    assert!(
        message.contains("looks damaged") && message.contains("docs/RECOVERY.md"),
        "message should point at recovery: {message}"
    );
    // The UI and agent surfaces both see the same stable kind.
    assert_eq!(
        ApplicationError::from(failure).kind(),
        "invalid_stored_data"
    );

    // The read-only helper path reports it the same way.
    let read_only = expect_open_failure(
        Storage::open_existing(&database_path),
        "damaged file stays closed",
    );
    assert!(matches!(read_only, StorageError::InvalidStoredData(_)));

    // And it is refused as a restore source rather than silently accepted.
    let as_backup =
        Storage::verify_backup_file(&database_path).expect_err("damaged file is not a backup");
    assert!(matches!(as_backup, StorageError::RestoreInvalid(_)));
}
