use contractorcrm_lib::{
    error::ApplicationError, storage, LOCAL_API_V1_COMMANDS, LOCAL_API_VERSION,
};
use serde_json::Value;

const DATA_MODEL_SCHEMA: &str = include_str!("../../schemas/v1/data-model.json");
const LOCAL_API_SCHEMA: &str = include_str!("../../schemas/v1/local-api.json");

fn string_array(value: &Value, field: &str) -> Vec<String> {
    value[field]
        .as_array()
        .unwrap_or_else(|| panic!("{field} must be an array"))
        .iter()
        .map(|item| {
            item.as_str()
                .unwrap_or_else(|| panic!("{field} entries must be strings"))
                .to_owned()
        })
        .collect()
}

#[test]
fn data_model_v1_matches_the_live_database_schema() {
    let schema: Value = serde_json::from_str(DATA_MODEL_SCHEMA).expect("valid data model schema");
    assert_eq!(schema["contract"], "contractorcrm-data-model");
    assert_eq!(schema["schemaVersion"], 1);
    assert_eq!(
        schema["databaseMigrationVersion"],
        storage::latest_migration_version()
    );

    let temp = tempfile::tempdir().expect("create temporary app data");
    let storage = storage::Storage::open_in_app_data(temp.path()).expect("open storage");
    let mut statement = storage
        .connection()
        .prepare(
            "SELECT name FROM sqlite_master
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE 'search_index_%'
             ORDER BY name",
        )
        .expect("prepare table listing");
    let actual_tables = statement
        .query_map([], |row| row.get(0))
        .expect("query table names")
        .collect::<rusqlite::Result<Vec<String>>>()
        .expect("collect table names");

    assert_eq!(string_array(&schema, "tables"), actual_tables);

    let migration_versions = schema["migrations"]
        .as_array()
        .expect("migrations must be an array")
        .iter()
        .map(|migration| migration["version"].as_i64().expect("integer version"))
        .collect::<Vec<_>>();
    assert_eq!(
        migration_versions,
        (1..=storage::latest_migration_version()).collect::<Vec<_>>()
    );
}

#[test]
fn local_api_v1_matches_the_registered_command_contract() {
    let schema: Value = serde_json::from_str(LOCAL_API_SCHEMA).expect("valid local API schema");
    assert_eq!(schema["contract"], "contractorcrm-local-api");
    assert_eq!(schema["apiVersion"], LOCAL_API_VERSION);
    assert_eq!(
        string_array(&schema, "commands"),
        LOCAL_API_V1_COMMANDS
            .iter()
            .map(|command| (*command).to_owned())
            .collect::<Vec<_>>()
    );

    let mut unique_commands = string_array(&schema, "commands");
    let command_count = unique_commands.len();
    unique_commands.sort();
    unique_commands.dedup();
    assert_eq!(
        unique_commands.len(),
        command_count,
        "duplicate API command"
    );

    let mut actual_error_kinds = vec![
        ApplicationError::InvalidInput {
            field: "field".into(),
            message: "invalid".into(),
        }
        .kind(),
        ApplicationError::ValidationFailed {
            code: "invalid",
            field: "field".into(),
            message: "invalid".into(),
        }
        .kind(),
        ApplicationError::NotFound {
            resource: "record",
            id: "id".into(),
        }
        .kind(),
        ApplicationError::MissingLostReason { id: "id".into() }.kind(),
        ApplicationError::VersionConflict {
            resource: "record",
            id: "id".into(),
            expected: 1,
            current: 2,
        }
        .kind(),
        ApplicationError::InvalidStoredData("invalid".into()).kind(),
        ApplicationError::BackupFailed("failed".into()).kind(),
        ApplicationError::RestoreInvalid("invalid".into()).kind(),
        ApplicationError::Database(rusqlite::Error::InvalidQuery).kind(),
        ApplicationError::Io(std::io::Error::other("failed")).kind(),
    ];
    actual_error_kinds.sort();
    let mut schema_error_kinds = string_array(&schema, "errorKinds");
    schema_error_kinds.sort();
    assert_eq!(schema_error_kinds, actual_error_kinds);
}
