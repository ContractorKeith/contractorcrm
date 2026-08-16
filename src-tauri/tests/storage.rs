use contractorcrm_lib::storage::{new_id, now_utc, Storage};
use rusqlite::params;

/// Table names the migrations must create, plus the migration ledger itself.
const EXPECTED_TABLES: &[&str] = &[
    "activities",
    "app_settings",
    "command_log",
    "companies",
    "contact_channels",
    "contacts",
    "lost_reasons",
    "opportunities",
    "pipelines",
    "schema_migrations",
    "search_index",
    "stage_history",
    "stages",
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
    assert_eq!(applied_versions(&storage), vec![1, 2, 3, 4, 5, 6]);
    drop(storage);

    // Reopening re-runs the migration framework; applied versions are skipped.
    let reopened = Storage::open(&database_path).expect("second open");
    assert_eq!(applied_versions(&reopened), vec![1, 2, 3, 4, 5, 6]);
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
