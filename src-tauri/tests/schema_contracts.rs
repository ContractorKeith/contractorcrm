use contractorcrm_lib::{
    application::{
        SavedView, SavedViewDefinition, SavedViewEntityType, SavedViewFilter, SavedViewSort,
        SavedViewSortDirection, SearchResult,
    },
    error::ApplicationError,
    storage, LOCAL_API_V1_COMMANDS, LOCAL_API_VERSION,
};
use std::collections::BTreeMap;

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

    let table_contracts = schema["tables"]
        .as_object()
        .expect("tables must be an object keyed by table name");
    assert_eq!(
        table_contracts.keys().cloned().collect::<Vec<_>>(),
        actual_tables
    );

    for (table_name, contract) in table_contracts {
        let escaped = table_name.replace('"', "\"\"");
        let mut columns_statement = storage
            .connection()
            .prepare(&format!("PRAGMA table_info(\"{escaped}\")"))
            .expect("prepare column listing");
        let column_rows = columns_statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)? == 1,
                    row.get::<_, i64>(5)? > 0,
                ))
            })
            .expect("query columns")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("collect columns");
        let actual_columns = column_rows
            .iter()
            .map(|(name, data_type, _, _)| {
                (
                    name.clone(),
                    if data_type.is_empty() {
                        "ANY".to_owned()
                    } else {
                        data_type.clone()
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let expected_columns = contract["columns"]
            .as_object()
            .expect("columns must be an object")
            .iter()
            .map(|(name, data_type)| {
                (
                    name.clone(),
                    data_type
                        .as_str()
                        .expect("column type must be text")
                        .to_owned(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(expected_columns, actual_columns, "columns for {table_name}");

        let actual_required = column_rows
            .iter()
            .filter(|(_, _, required, _)| *required)
            .map(|(name, _, _, _)| name.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            string_array(contract, "required"),
            actual_required,
            "required columns for {table_name}"
        );
        let actual_primary_key = column_rows
            .iter()
            .filter(|(_, _, _, primary_key)| *primary_key)
            .map(|(name, _, _, _)| name.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            string_array(contract, "primaryKey"),
            actual_primary_key,
            "primary key for {table_name}"
        );

        let mut foreign_key_statement = storage
            .connection()
            .prepare(&format!("PRAGMA foreign_key_list(\"{escaped}\")"))
            .expect("prepare foreign key listing");
        let mut actual_foreign_keys = foreign_key_statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(6)?,
                ))
            })
            .expect("query foreign keys")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("collect foreign keys");
        actual_foreign_keys.sort();
        let mut expected_foreign_keys = contract
            .get("foreignKeys")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|foreign_key| {
                (
                    foreign_key["from"]
                        .as_str()
                        .expect("foreign key from")
                        .to_owned(),
                    foreign_key["table"]
                        .as_str()
                        .expect("foreign key table")
                        .to_owned(),
                    foreign_key["to"]
                        .as_str()
                        .expect("foreign key to")
                        .to_owned(),
                    foreign_key["onDelete"]
                        .as_str()
                        .expect("foreign key onDelete")
                        .to_owned(),
                )
            })
            .collect::<Vec<_>>();
        expected_foreign_keys.sort();
        assert_eq!(
            expected_foreign_keys, actual_foreign_keys,
            "foreign keys for {table_name}"
        );

        if let Some(checks) = contract.get("sqlChecks") {
            let create_sql: String = storage
                .connection()
                .query_row(
                    "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [table_name],
                    |row| row.get(0),
                )
                .expect("load create SQL");
            for check in checks.as_array().expect("sqlChecks must be an array") {
                let check = check.as_str().expect("sqlChecks entries must be text");
                assert!(
                    create_sql.contains(check),
                    "{table_name} must retain constraint: {check}"
                );
            }
        }
    }

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
    let commands = schema["commands"]
        .as_array()
        .expect("commands must be an array");
    let command_names = commands
        .iter()
        .map(|command| {
            command["name"]
                .as_str()
                .expect("each command needs a name")
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        command_names,
        LOCAL_API_V1_COMMANDS
            .iter()
            .map(|command| (*command).to_owned())
            .collect::<Vec<_>>()
    );

    for command in commands {
        assert!(matches!(
            command["mode"].as_str(),
            Some("read") | Some("write")
        ));
        assert!(command["output"]
            .as_str()
            .is_some_and(|value| !value.is_empty()));
        for input in command["input"].as_array().expect("input must be an array") {
            assert!(input["name"]
                .as_str()
                .is_some_and(|value| !value.is_empty()));
            assert!(input["type"]
                .as_str()
                .is_some_and(|value| !value.is_empty()));
            assert!(input["required"].is_boolean());
        }
    }

    let mut unique_commands = command_names;
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

#[test]
fn search_result_v1_matches_the_published_wire_schema() {
    let schema: Value = serde_json::from_str(LOCAL_API_SCHEMA).expect("valid local API schema");
    let serialized = serde_json::to_value(SearchResult {
        entity_type: "activity".into(),
        entity_id: "activity-1".into(),
        title: "Called customer".into(),
        parent_type: Some("contact".into()),
        parent_id: Some("contact-1".into()),
    })
    .expect("serialize search result");
    let actual_fields = serialized
        .as_object()
        .expect("search result is an object")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    let mut expected_fields = string_array(&schema["wireTypes"]["SearchResult"], "required");
    expected_fields.sort();
    let mut actual_fields = actual_fields;
    actual_fields.sort();
    assert_eq!(expected_fields, actual_fields);
    assert_eq!(serialized["parentType"], "contact");
    assert_eq!(serialized["parentId"], "contact-1");
}

#[test]
fn navigation_commands_are_additive_v1_contract_entries() {
    let schema: Value = serde_json::from_str(LOCAL_API_SCHEMA).expect("valid local API schema");
    let commands = schema["commands"].as_array().expect("commands array");
    let names = commands
        .iter()
        .map(|command| command["name"].as_str().expect("command name"))
        .collect::<Vec<_>>();
    assert!(names.contains(&"list_recent_records"));
    assert!(names.contains(&"record_recent"));
    assert!(names.contains(&"list_favorite_contacts"));
    assert_eq!(
        string_array(&schema["wireTypes"]["NavigationEntityType"], "enum"),
        ["contact", "company", "opportunity"]
    );
}

#[test]
fn saved_view_v1_matches_the_published_wire_schema() {
    let schema: Value = serde_json::from_str(LOCAL_API_SCHEMA).expect("valid local API schema");
    let saved_view = SavedView {
        id: "view-1".into(),
        name: "Active prospects".into(),
        entity_type: SavedViewEntityType::Opportunity,
        definition: SavedViewDefinition {
            schema_version: 1,
            filter: SavedViewFilter {
                include_archived: false,
            },
            sort: SavedViewSort {
                field: "expectedClose".into(),
                direction: SavedViewSortDirection::Ascending,
            },
        },
        sort_key: 0,
        created_at: "2026-08-16T00:00:00.000Z".into(),
        updated_at: "2026-08-16T00:00:00.000Z".into(),
        version: 1,
    };
    let serialized = serde_json::to_value(saved_view).expect("serialize saved view");
    let mut actual_fields = serialized
        .as_object()
        .expect("saved view is an object")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    actual_fields.sort();
    let mut expected_fields = string_array(&schema["wireTypes"]["SavedView"], "required");
    expected_fields.sort();
    assert_eq!(actual_fields, expected_fields);
    assert_eq!(serialized["entityType"], "opportunity");
    assert_eq!(serialized["definition"]["schemaVersion"], 1);
    assert_eq!(serialized["definition"]["sort"]["direction"], "ascending");
    for strict_type in ["SavedViewFilter", "SavedViewSort", "SavedViewDefinition"] {
        assert_eq!(
            schema["wireTypes"][strict_type]["additionalProperties"], false,
            "{strict_type} must reject unknown fields"
        );
    }
    assert_eq!(
        string_array(&schema["wireTypes"]["SavedViewSortField"], "enum"),
        ["displayName", "name", "stage", "value", "expectedClose"]
    );
}
