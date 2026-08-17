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
    assert_eq!(applied_versions(&storage), vec![1, 2, 3, 4, 5, 6, 7, 8]);
    drop(storage);

    // Reopening re-runs the migration framework; applied versions are skipped.
    let reopened = Storage::open(&database_path).expect("second open");
    assert_eq!(applied_versions(&reopened), vec![1, 2, 3, 4, 5, 6, 7, 8]);
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
         DELETE FROM schema_migrations WHERE version=8;",
        )
        .expect("return fixture to populated v7");
    drop(storage);

    let upgraded = Storage::open(&database_path).expect("upgrade v7 to v8");
    assert_eq!(applied_versions(&upgraded), vec![1, 2, 3, 4, 5, 6, 7, 8]);
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
    assert_eq!(applied_versions(&reopened), vec![1, 2, 3, 4, 5, 6, 7, 8]);
}
