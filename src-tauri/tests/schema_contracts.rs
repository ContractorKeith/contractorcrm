use contractorcrm_lib::{
    ai::{AiSettings, ProviderCheck, ProviderCompletion, RecordRef, SetAiSettingsRequest},
    application::{
        ContactImportMapping, CreateTagRequest, CustomFieldValueInput, ImportContactsRequest,
        ProductInfo, SavedView, SavedViewDefinition, SavedViewEntityType, SavedViewFilter,
        SavedViewSort, SavedViewSortDirection, SearchResult, SetRecordMetadataRequest,
    },
    archive::{ArchiveFileEntry, ArchiveIssue, ArchiveManifest, ARCHIVE_SCHEMA_VERSION},
    attachments::{
        AddAttachmentRequest, Attachment, AttachmentParentType, RemoveAttachmentRequest,
    },
    error::ApplicationError,
    explain::AttentionExplanation,
    followups::{
        FollowupDraft, FollowupTemplate, FollowupTemplates, HistorySummary,
        SetFollowupTemplatesRequest,
    },
    proposals::{
        ApplyProposalRequest, FieldChange, Proposal, ProposalApplied, ProposalEntityType,
        ProposalKind, ProposalUndone, RecordVersion, UndoProposalRequest,
    },
    storage, PreviewContextRequest, LOCAL_API_V1_COMMANDS, LOCAL_API_VERSION,
};
use std::collections::BTreeMap;

use serde_json::Value;

const DATA_MODEL_SCHEMA: &str = include_str!("../../schemas/v1/data-model.json");
const LOCAL_API_SCHEMA: &str = include_str!("../../schemas/v1/local-api.json");
const HANDOFF_ENVELOPE_SCHEMA: &str = include_str!("../../schemas/v1/handoff-envelope.json");

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

        if let Some(indexes) = contract.get("indexes").and_then(Value::as_array) {
            let mut index_list = storage
                .connection()
                .prepare(&format!("PRAGMA index_list(\"{escaped}\")"))
                .expect("prepare index listing");
            let actual_indexes = index_list
                .query_map([], |row| {
                    Ok((row.get::<_, String>(1)?, row.get::<_, i64>(2)? == 1))
                })
                .expect("query indexes")
                .collect::<rusqlite::Result<BTreeMap<_, _>>>()
                .expect("collect indexes");
            for index in indexes {
                let name = index["name"].as_str().expect("index name");
                let unique = index["unique"].as_bool().expect("index unique flag");
                assert_eq!(
                    actual_indexes.get(name),
                    Some(&unique),
                    "index uniqueness for {name}"
                );
                let escaped_index = name.replace('"', "\"\"");
                let mut info = storage
                    .connection()
                    .prepare(&format!("PRAGMA index_info(\"{escaped_index}\")"))
                    .expect("prepare index columns");
                let columns = info
                    .query_map([], |row| row.get::<_, String>(2))
                    .expect("query index columns")
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .expect("collect index columns");
                assert_eq!(
                    columns,
                    string_array(index, "columns"),
                    "index columns for {name}"
                );
            }
        }

        if let Some(checks) = contract.get("sqlChecks") {
            let create_sql: String = storage
                .connection()
                .query_row(
                    "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [table_name],
                    |row| row.get(0),
                )
                .expect("load create SQL");
            let normalized_create_sql = create_sql.split_whitespace().collect::<Vec<_>>().join(" ");
            for check in checks.as_array().expect("sqlChecks must be an array") {
                let check = check.as_str().expect("sqlChecks entries must be text");
                let normalized_check = check.split_whitespace().collect::<Vec<_>>().join(" ");
                assert!(
                    normalized_create_sql.contains(&normalized_check),
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
    let mut actual_triggers = storage
        .connection()
        .prepare("SELECT name FROM sqlite_master WHERE type='trigger' ORDER BY name")
        .expect("prepare triggers")
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query triggers")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect triggers");
    let mut expected_triggers = string_array(&schema, "triggers");
    actual_triggers.sort();
    expected_triggers.sort();
    assert_eq!(actual_triggers, expected_triggers);
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
        ApplicationError::ProviderUnavailable {
            reason: "unreachable".into(),
        }
        .kind(),
        ApplicationError::ProposalExpired {
            proposal_id: "proposal-1".into(),
        }
        .kind(),
        ApplicationError::ReadOnly {
            command: "apply_proposal".into(),
        }
        .kind(),
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
            schema_version: 2,
            filter: SavedViewFilter {
                include_archived: false,
                tag_ids_all: vec![],
                custom_fields: vec![],
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
    assert_eq!(serialized["definition"]["schemaVersion"], 2);
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

#[test]
fn tags_and_custom_fields_publish_strict_bounded_wire_types() {
    let schema: Value = serde_json::from_str(LOCAL_API_SCHEMA).expect("valid local API schema");
    let commands = schema["commands"].as_array().expect("commands array");
    for name in [
        "list_tags",
        "create_tag",
        "update_tag",
        "archive_tag",
        "unarchive_tag",
        "list_custom_field_defs",
        "create_custom_field_def",
        "update_custom_field_def",
        "archive_custom_field_def",
        "unarchive_custom_field_def",
        "get_record_metadata",
        "set_record_metadata",
        "match_saved_view",
    ] {
        assert!(
            commands.iter().any(|command| command["name"] == name),
            "missing {name}"
        );
    }
    for strict_type in [
        "Tag",
        "CreateTagRequest",
        "UpdateTagRequest",
        "TagLifecycleRequest",
        "CustomFieldOption",
        "CustomFieldOptionInput",
        "CustomFieldDefinition",
        "CreateCustomFieldDefinitionRequest",
        "UpdateCustomFieldDefinitionRequest",
        "CustomFieldDefinitionLifecycleRequest",
        "RecordCustomFieldValue",
        "CustomFieldValueInput",
        "RecordMetadata",
        "SetRecordMetadataRequest",
        "SavedViewCustomFieldPredicate",
    ] {
        assert_eq!(
            schema["wireTypes"][strict_type]["additionalProperties"], false,
            "{strict_type} must reject unknown fields"
        );
    }
    assert_eq!(
        string_array(&schema["wireTypes"]["TagColorRole"], "enum"),
        ["neutral", "accent", "attention"]
    );
    assert_eq!(
        string_array(&schema["wireTypes"]["CustomFieldType"], "enum"),
        ["text", "number", "date", "select"]
    );

    assert!(
        serde_json::from_value::<CreateTagRequest>(serde_json::json!({
            "label": "Priority", "colorRole": "accent", "rawSql": "DROP TABLE tags"
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<SetRecordMetadataRequest>(serde_json::json!({
            "entityType": "company", "recordId": "company-1", "expectedVersion": 1,
            "tagIds": [], "values": [], "unexpected": true
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<SetRecordMetadataRequest>(serde_json::json!({
            "entityType": "company", "recordId": "company-1", "expectedVersion": 1
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<CustomFieldValueInput>(serde_json::json!({
            "definitionId": "field-1", "textValue": "value"
        }))
        .is_err()
    );
}

#[test]
fn portable_archive_publishes_strict_bounded_wire_types() {
    let schema: Value = serde_json::from_str(LOCAL_API_SCHEMA).expect("valid local API schema");
    let commands = schema["commands"].as_array().expect("commands array");
    for name in ["export_archive", "preview_archive_import", "import_archive"] {
        assert!(
            commands.iter().any(|command| command["name"] == name),
            "missing {name}"
        );
    }
    for strict_type in [
        "ProductInfo",
        "ArchiveFileEntry",
        "ArchiveManifest",
        "ArchiveIssue",
        "ArchiveExportReport",
        "ArchiveImportPreview",
        "ArchiveImportReport",
    ] {
        assert_eq!(
            schema["wireTypes"][strict_type]["additionalProperties"], false,
            "{strict_type} must reject unknown fields"
        );
    }
    // The published manifest shape is exactly what the exporter writes.
    let manifest = serde_json::to_value(ArchiveManifest {
        schema_version: ARCHIVE_SCHEMA_VERSION,
        product: ProductInfo {
            name: "ContractorCRM".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
        exported_at: "2026-08-18T00:00:00.000Z".into(),
        database_migration_version: storage::latest_migration_version(),
        files: vec![ArchiveFileEntry {
            path: "data/contacts.json".into(),
            sha256: "0".repeat(64),
            bytes: 2,
        }],
        record_counts: BTreeMap::from([("contacts".to_owned(), 1)]),
    })
    .expect("serialize manifest");
    let mut actual_fields = manifest
        .as_object()
        .expect("manifest is an object")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    actual_fields.sort();
    let mut expected_fields = string_array(&schema["wireTypes"]["ArchiveManifest"], "required");
    expected_fields.sort();
    assert_eq!(actual_fields, expected_fields);
    assert_eq!(
        schema["wireTypes"]["ArchiveManifest"]["properties"]["schemaVersion"]["const"],
        ARCHIVE_SCHEMA_VERSION
    );
    assert_eq!(
        manifest["files"][0]["sha256"].as_str().map(str::len),
        Some(64)
    );

    assert!(serde_json::from_value::<ArchiveIssue>(serde_json::json!({
        "code": "checksum_mismatch", "message": "bad", "unexpected": true
    }))
    .is_err());
}

#[test]
fn attachments_publish_strict_bounded_wire_types() {
    let schema: Value = serde_json::from_str(LOCAL_API_SCHEMA).expect("valid local API schema");
    let commands = schema["commands"].as_array().expect("commands array");
    for name in [
        "add_attachment",
        "list_attachments",
        "remove_attachment",
        "attachment_path",
    ] {
        assert!(
            commands.iter().any(|command| command["name"] == name),
            "missing {name}"
        );
    }
    for strict_type in [
        "Attachment",
        "AddAttachmentRequest",
        "RemoveAttachmentRequest",
        "AttachmentRemoval",
        "AttachmentLocation",
    ] {
        assert_eq!(
            schema["wireTypes"][strict_type]["additionalProperties"], false,
            "{strict_type} must reject unknown fields"
        );
    }
    assert_eq!(
        string_array(&schema["wireTypes"]["AttachmentParentType"], "enum"),
        ["contact", "opportunity"]
    );

    // The published shape is exactly what the seam returns; the managed
    // relative path stays internal.
    let attachment = Attachment {
        id: "attachment-1".into(),
        parent_type: AttachmentParentType::Opportunity,
        parent_id: "opportunity-1".into(),
        file_name: "cedar-quote.pdf".into(),
        media_type: Some("application/pdf".into()),
        size_bytes: 12,
        sha256: "0".repeat(64),
        created_at: "2026-08-18T00:00:00.000Z".into(),
        version: 1,
    };
    let serialized = serde_json::to_value(attachment).expect("serialize attachment");
    let mut actual_fields = serialized
        .as_object()
        .expect("attachment is an object")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    actual_fields.sort();
    let mut expected_fields = string_array(&schema["wireTypes"]["Attachment"], "required");
    expected_fields.sort();
    assert_eq!(actual_fields, expected_fields);
    assert_eq!(serialized["parentType"], "opportunity");
    assert!(serialized.get("relativePath").is_none());

    assert!(
        serde_json::from_value::<AddAttachmentRequest>(serde_json::json!({
            "parentType": "contact", "parentId": "contact-1",
            "sourcePath": "/tmp/quote.pdf", "unexpected": true
        }))
        .is_err()
    );
    // Attachments hang off contacts and opportunities only.
    assert!(
        serde_json::from_value::<AddAttachmentRequest>(serde_json::json!({
            "parentType": "company", "parentId": "company-1", "sourcePath": "/tmp/quote.pdf"
        }))
        .is_err()
    );
    // Adds default to the user actor recorded in the command log.
    let defaulted: AddAttachmentRequest = serde_json::from_value(serde_json::json!({
        "parentType": "contact", "parentId": "contact-1", "sourcePath": "/tmp/quote.pdf"
    }))
    .expect("actor is optional");
    assert_eq!(
        serde_json::to_value(defaulted.actor).expect("serialize actor"),
        "user"
    );
    assert!(
        serde_json::from_value::<RemoveAttachmentRequest>(serde_json::json!({
            "attachmentId": "attachment-1"
        }))
        .is_err()
    );
}

#[test]
fn csv_import_export_publish_strict_bounded_wire_types() {
    let schema: Value = serde_json::from_str(LOCAL_API_SCHEMA).expect("valid local API schema");
    let commands = schema["commands"].as_array().expect("commands array");
    for name in [
        "preview_contact_import",
        "import_contacts",
        "export_contacts_csv",
        "export_opportunities_csv",
    ] {
        assert!(
            commands.iter().any(|command| command["name"] == name),
            "missing {name}"
        );
    }
    for strict_type in [
        "ContactImportMapping",
        "ContactImportIssue",
        "ContactImportPreview",
        "ImportContactsRequest",
        "ContactImportSummary",
        "CsvExportReport",
    ] {
        assert_eq!(
            schema["wireTypes"][strict_type]["additionalProperties"], false,
            "{strict_type} must reject unknown fields"
        );
    }
    // The published mapping targets are exactly the fields the seam accepts.
    let mut published = string_array(&schema["wireTypes"]["ContactImportTarget"], "enum");
    published.sort();
    let mut mapping_properties = schema["wireTypes"]["ContactImportMapping"]["properties"]
        .as_object()
        .expect("mapping properties")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    mapping_properties.sort();
    assert_eq!(published, mapping_properties);
    let round_trip: ContactImportMapping = serde_json::from_value(serde_json::json!({
        "firstName": "First", "lastName": "Last", "email": "Email"
    }))
    .expect("partial mapping deserializes");
    assert_eq!(round_trip.first_name.as_deref(), Some("First"));
    assert!(round_trip.display_name.is_none());

    assert!(
        serde_json::from_value::<ContactImportMapping>(serde_json::json!({
            "firstName": "First", "customField": "Nope"
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<ImportContactsRequest>(serde_json::json!({
            "path": "/tmp/contacts.csv", "mapping": {}, "unexpected": true
        }))
        .is_err()
    );
    // Imports default to the `import` actor recorded in the command log.
    let defaulted: ImportContactsRequest = serde_json::from_value(serde_json::json!({
        "path": "/tmp/contacts.csv", "mapping": {}
    }))
    .expect("actor is optional");
    assert_eq!(
        serde_json::to_value(defaulted.actor).expect("serialize actor"),
        "import"
    );
}

#[test]
fn ai_provider_commands_and_wire_types_match_the_published_v1_contract() {
    let schema: Value = serde_json::from_str(LOCAL_API_SCHEMA).expect("valid local API schema");
    let names = schema["commands"]
        .as_array()
        .expect("commands array")
        .iter()
        .map(|command| command["name"].as_str().expect("command name"))
        .collect::<Vec<_>>();
    for command in [
        "get_ai_settings",
        "set_ai_settings",
        "set_ai_api_key",
        "clear_ai_api_key",
        "test_ai_provider",
    ] {
        assert!(names.contains(&command), "{command} must be published");
    }

    // The settings wire shape carries the derived has-key flag and never the key.
    let settings = serde_json::to_value(AiSettings {
        version: 1,
        enabled: true,
        provider_label: "Local model".into(),
        base_url: "http://127.0.0.1:11434/v1".into(),
        model: "llama3.1".into(),
        has_api_key: true,
    })
    .expect("serialize ai settings");
    let mut actual_fields = settings
        .as_object()
        .expect("settings object")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    actual_fields.sort();
    let mut expected_fields = string_array(&schema["wireTypes"]["AiSettings"], "required");
    expected_fields.sort();
    assert_eq!(expected_fields, actual_fields);
    assert!(!settings.to_string().contains("apiKey"));

    let check = serde_json::to_value(ProviderCheck {
        provider_label: "Local model".into(),
        endpoint_host: "127.0.0.1:11434".into(),
        local: true,
        model: "llama3.1".into(),
        model_available: true,
        available_models: vec!["llama3.1".into()],
    })
    .expect("serialize provider check");
    let mut actual_check_fields = check
        .as_object()
        .expect("check object")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    actual_check_fields.sort();
    let mut expected_check_fields = string_array(&schema["wireTypes"]["ProviderCheck"], "required");
    expected_check_fields.sort();
    assert_eq!(expected_check_fields, actual_check_fields);

    // Requests reject unknown fields and default the actor like every other write.
    let request: SetAiSettingsRequest = serde_json::from_value(serde_json::json!({
        "enabled": false,
        "providerLabel": "Local model",
        "baseUrl": "http://127.0.0.1:11434/v1",
        "model": ""
    }))
    .expect("actor is optional");
    assert_eq!(
        serde_json::to_value(request.actor).expect("serialize actor"),
        "user"
    );
    assert!(
        serde_json::from_value::<SetAiSettingsRequest>(serde_json::json!({
            "enabled": false,
            "providerLabel": "Local model",
            "baseUrl": "http://127.0.0.1:11434/v1",
            "model": "",
            "apiKey": "sk-nope"
        }))
        .is_err()
    );
}

#[test]
fn proposal_commands_and_wire_types_match_the_published_v1_contract() {
    let schema: Value = serde_json::from_str(LOCAL_API_SCHEMA).expect("valid local API schema");
    let commands = schema["commands"].as_array().expect("commands array");
    for (command, mode) in [
        ("propose_record", "read"),
        ("propose_update", "read"),
        ("apply_proposal", "write"),
        ("undo_proposal", "write"),
    ] {
        let published = commands
            .iter()
            .find(|entry| entry["name"] == command)
            .unwrap_or_else(|| panic!("{command} must be published"));
        // Drafting is published as a read: only apply/undo ever write.
        assert_eq!(published["mode"], mode, "{command} mode");
    }

    let proposal = Proposal {
        id: "proposal-1".into(),
        kind: ProposalKind::UpdateContact,
        entity_type: ProposalEntityType::Contact,
        entity_id: Some("contact-1".into()),
        summary: "Update contact \"Dana Ruiz\"".into(),
        changes: vec![FieldChange {
            field: "city".into(),
            before: Some("Sanford".into()),
            after: Some("Orlando".into()),
        }],
        warnings: vec!["Filed as a lead — change the kind if that's not right.".into()],
        affected_versions: vec![RecordVersion {
            entity_type: "contact".into(),
            entity_id: "contact-1".into(),
            version: 3,
        }],
        created_at: "2026-08-19T12:00:00.000Z".into(),
        expires_at: "2026-08-19T12:15:00.000Z".into(),
    };
    assert_published_shape(&schema, "Proposal", &proposal);
    assert_published_shape(&schema, "FieldChange", &proposal.changes[0]);
    assert_published_shape(&schema, "RecordVersion", &proposal.affected_versions[0]);
    assert_published_shape(
        &schema,
        "ProposalApplied",
        &ProposalApplied {
            entity_type: ProposalEntityType::Contact,
            entity_id: "contact-1".into(),
            created: false,
            version: 4,
            undo_token: "undo-1".into(),
            undo_expires_at: "2026-08-19T12:15:00.000Z".into(),
        },
    );
    assert_published_shape(
        &schema,
        "ProposalUndone",
        &ProposalUndone {
            entity_type: ProposalEntityType::Contact,
            entity_id: "contact-1".into(),
            action: "reverted".into(),
            version: 5,
        },
    );

    // Published enum values are the wire strings the core actually emits.
    assert_eq!(
        string_array(&schema["wireTypes"]["ProposalEntityType"], "enum"),
        vec!["contact", "company", "opportunity", "task"]
    );
    let published_kinds = string_array(&schema["wireTypes"]["ProposalKind"], "enum");
    for kind in [
        ProposalKind::CreateContact,
        ProposalKind::CreateCompany,
        ProposalKind::CreateOpportunity,
        ProposalKind::UpdateContact,
        ProposalKind::UpdateCompany,
        ProposalKind::UpdateOpportunity,
        ProposalKind::CreateFollowupTask,
    ] {
        let wire = serde_json::to_value(kind).expect("serialize kind");
        assert!(
            published_kinds.contains(&wire.as_str().expect("kind is a string").to_owned()),
            "{wire} must be published"
        );
    }

    // Apply/undo requests default the actor and reject unknown fields, like
    // every other write request.
    let request: ApplyProposalRequest =
        serde_json::from_value(serde_json::json!({"proposalId": "proposal-1"}))
            .expect("actor and versions are optional");
    assert_eq!(
        serde_json::to_value(request.actor).expect("serialize actor"),
        "user"
    );
    assert!(request.expected_versions.is_empty());
    assert!(serde_json::from_value::<ApplyProposalRequest>(
        serde_json::json!({"proposalId": "proposal-1", "force": true})
    )
    .is_err());
    let undo: UndoProposalRequest =
        serde_json::from_value(serde_json::json!({"undoToken": "undo-1"}))
            .expect("actor and versions are optional");
    assert_eq!(undo.undo_token, "undo-1");
}

/// Every field the core serializes is published, and nothing more.
fn assert_published_shape<T: serde::Serialize>(schema: &Value, name: &str, value: &T) {
    let serialized = serde_json::to_value(value).expect("serialize wire value");
    let mut actual = serialized
        .as_object()
        .unwrap_or_else(|| panic!("{name} is an object"))
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    actual.sort();
    let mut expected = string_array(&schema["wireTypes"][name], "required");
    expected.sort();
    assert_eq!(expected, actual, "{name} wire shape");
    assert_eq!(schema["wireTypes"][name]["additionalProperties"], false);
}

#[test]
fn attention_explanations_match_the_published_v1_contract() {
    let schema: Value = serde_json::from_str(LOCAL_API_SCHEMA).expect("valid local API schema");
    let command = schema["commands"]
        .as_array()
        .expect("commands array")
        .iter()
        .find(|command| command["name"] == "explain_attention_flag")
        .expect("explain_attention_flag must be published");
    assert_eq!(command["mode"], "read");
    assert_eq!(command["output"], "AttentionExplanation");
    assert_eq!(command["input"][0]["name"], "flagId");

    let explanation = serde_json::to_value(AttentionExplanation {
        flag_id: "stale_lead:contact-1".into(),
        endpoint_host: "127.0.0.1:11434".into(),
        local: true,
        explanation: ProviderCompletion {
            purpose: "explain_attention_flag".into(),
            model: "llama3.1".into(),
            text: "Dana has gone quiet — call this week.".into(),
            included_record_refs: vec![RecordRef {
                entity_type: "contact".into(),
                entity_id: "contact-1".into(),
                label: "Dana Ruiz".into(),
            }],
        },
    })
    .expect("serialize attention explanation");

    for (wire_type, actual) in [
        ("AttentionExplanation", &explanation),
        ("ProviderCompletion", &explanation["explanation"]),
    ] {
        let mut actual_fields = actual
            .as_object()
            .unwrap_or_else(|| panic!("{wire_type} object"))
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        actual_fields.sort();
        let mut expected_fields = string_array(&schema["wireTypes"][wire_type], "required");
        expected_fields.sort();
        assert_eq!(expected_fields, actual_fields, "{wire_type}");
        assert_eq!(
            schema["wireTypes"][wire_type]["additionalProperties"],
            false
        );
    }
}

#[test]
fn followup_commands_and_wire_types_match_the_published_v1_contract() {
    let schema: Value = serde_json::from_str(LOCAL_API_SCHEMA).expect("valid local API schema");
    let commands = schema["commands"].as_array().expect("commands array");
    for (command, mode, output) in [
        ("get_followup_templates", "read", "FollowupTemplates"),
        ("set_followup_templates", "write", "FollowupTemplates"),
        // Summarizing and drafting only read; only apply_proposal writes.
        ("summarize_history", "read", "HistorySummary"),
        ("propose_followup", "read", "FollowupDraft"),
    ] {
        let published = commands
            .iter()
            .find(|entry| entry["name"] == command)
            .unwrap_or_else(|| panic!("{command} must be published"));
        assert_eq!(published["mode"], mode, "{command} mode");
        assert_eq!(published["output"], output, "{command} output");
    }

    let template = FollowupTemplate {
        id: "call_followup".into(),
        name: "Call follow-up".into(),
        body: "Thanks for taking the time on the phone.".into(),
    };
    assert_published_shape(&schema, "FollowupTemplate", &template);
    assert_published_shape(
        &schema,
        "FollowupTemplates",
        &FollowupTemplates {
            version: 1,
            templates: vec![template.clone()],
        },
    );
    assert_published_shape(
        &schema,
        "HistorySummary",
        &HistorySummary {
            parent_type: "contact".into(),
            parent_id: "contact-1".into(),
            endpoint_host: "127.0.0.1:11434".into(),
            local: true,
            model: "llama3.1".into(),
            summary: "Dana asked for a gate quote and went quiet.".into(),
            suggested_next_actions: vec!["Call Dana this week".into()],
            included_record_refs: vec![RecordRef {
                entity_type: "contact".into(),
                entity_id: "contact-1".into(),
                label: "Dana Ruiz".into(),
            }],
        },
    );
    assert_published_shape(
        &schema,
        "FollowupDraft",
        &FollowupDraft {
            parent_type: "contact".into(),
            parent_id: "contact-1".into(),
            template_id: template.id.clone(),
            template_name: template.name.clone(),
            draft_text: template.body.clone(),
            used_provider: false,
            endpoint_host: None,
            local: false,
            model: None,
            included_record_refs: Vec::new(),
            warnings: Vec::new(),
            proposal: Proposal {
                id: "proposal-1".into(),
                kind: ProposalKind::CreateFollowupTask,
                entity_type: ProposalEntityType::Task,
                entity_id: None,
                summary: "Follow up with Dana Ruiz".into(),
                changes: Vec::new(),
                warnings: Vec::new(),
                affected_versions: Vec::new(),
                created_at: "2026-08-19T12:00:00.000Z".into(),
                expires_at: "2026-08-19T12:15:00.000Z".into(),
            },
        },
    );

    // A set request defaults the actor and refuses unknown fields.
    let request: SetFollowupTemplatesRequest =
        serde_json::from_value(serde_json::json!({"templates": []})).expect("actor is optional");
    assert_eq!(
        serde_json::to_value(request.actor).expect("serialize actor"),
        "user"
    );
    assert!(serde_json::from_value::<SetFollowupTemplatesRequest>(
        serde_json::json!({"templates": [], "force": true})
    )
    .is_err());
}

/// `preview_context` is the desktop's "see what will be sent" seam: one command
/// covering every AI-backed feature, each arm parsed from the same arguments
/// that feature's own command takes.
#[test]
fn preview_context_publishes_one_arm_per_ai_backed_feature() {
    let schema: Value = serde_json::from_str(LOCAL_API_SCHEMA).expect("valid local API schema");
    let published = schema["commands"]
        .as_array()
        .expect("commands array")
        .iter()
        .find(|command| command["name"] == "preview_context")
        .expect("preview_context must be published");
    assert_eq!(published["mode"], "read");
    assert_eq!(published["output"], "ContextPreview");
    assert_eq!(published["input"][0]["name"], "request");
    assert_eq!(published["input"][0]["type"], "PreviewContextRequest");

    let mut published_tools = string_array(
        &schema["wireTypes"]["PreviewContextRequest"]["properties"]["tool"],
        "enum",
    );
    published_tools.sort();

    // Every published arm round-trips from the wire shape a client sends.
    let requests = [
        serde_json::json!({
            "tool": "summarize_history",
            "parentType": "contact",
            "parentId": "contact-1",
            "window": 30,
        }),
        serde_json::json!({"tool": "explain_attention_flag", "flagId": "stale_lead:contact-1"}),
        serde_json::json!({
            "tool": "propose_update",
            "entityType": "contact",
            "entityId": "contact-1",
            "expectedVersion": 3,
        }),
        serde_json::json!({
            "tool": "propose_followup",
            "parentType": "contact",
            "parentId": "contact-1",
            "objective": "chase the proposal",
            "templateId": "proposal_chaser",
        }),
    ];
    let mut round_tripped = Vec::new();
    for request in requests {
        let parsed: PreviewContextRequest =
            serde_json::from_value(request.clone()).expect("a published arm parses");
        let reserialized = serde_json::to_value(&parsed).expect("serialize the arm");
        assert_eq!(reserialized, request, "{request} round trip");
        round_tripped.push(request["tool"].as_str().expect("a tool name").to_owned());
    }
    round_tripped.sort();
    assert_eq!(round_tripped, published_tools);

    // The optional arguments really are optional, and an unknown tool is not
    // silently treated as one of the four.
    serde_json::from_value::<PreviewContextRequest>(serde_json::json!({
        "tool": "summarize_history",
        "parentType": "contact",
        "parentId": "contact-1",
    }))
    .expect("window is optional");
    assert!(serde_json::from_value::<PreviewContextRequest>(
        serde_json::json!({"tool": "list_contacts"})
    )
    .is_err());

    assert_published_shape(
        &schema,
        "ContextPreview",
        &contractorcrm_lib::ai::ContextPreview {
            purpose: "summarize_history".into(),
            context_text: "Record: Dana Ruiz (contact contact-1)".into(),
            included_record_refs: vec![RecordRef {
                entity_type: "contact".into(),
                entity_id: "contact-1".into(),
                label: "Dana Ruiz".into(),
            }],
        },
    );
}

// Hand-off envelope contract (schemas/v1/handoff-envelope.json)

/// Compare one exported object against a named manifest type. Manifest fields
/// are required and unknown exported fields fail, so any envelope addition has
/// to be a deliberate manifest edit.
fn assert_envelope_type(types: &Value, type_name: &str, value: &Value, path: &str) {
    let definition = &types[type_name];
    assert!(
        definition.is_object(),
        "{path}: the manifest declares no type {type_name}"
    );
    assert_eq!(
        definition["additionalProperties"], false,
        "{type_name} must reject unknown fields"
    );
    let object = value
        .as_object()
        .unwrap_or_else(|| panic!("{path} must be a JSON object"));
    let mut actual_fields = object.keys().cloned().collect::<Vec<_>>();
    actual_fields.sort();
    let mut expected_fields = string_array(definition, "required");
    expected_fields.sort();
    assert_eq!(
        actual_fields, expected_fields,
        "{path} fields must match the {type_name} manifest exactly"
    );
    let properties = definition["properties"]
        .as_object()
        .unwrap_or_else(|| panic!("{type_name} must declare properties"));
    let mut described_fields = properties.keys().cloned().collect::<Vec<_>>();
    described_fields.sort();
    assert_eq!(
        described_fields, expected_fields,
        "{type_name} must describe every required field"
    );
    for (field, property) in properties {
        assert_envelope_value(types, property, &object[field], &format!("{path}.{field}"));
    }
}

/// Check one exported value against its manifest property description.
fn assert_envelope_value(types: &Value, property: &Value, value: &Value, path: &str) {
    let declared = property["type"]
        .as_str()
        .unwrap_or_else(|| panic!("{path} needs a declared manifest type"));
    if value.is_null() {
        assert_eq!(
            property["nullable"],
            Value::Bool(true),
            "{path} is not nullable in the manifest"
        );
        return;
    }
    match declared {
        "string" => {
            assert!(value.is_string(), "{path} must be a string");
            if let Some(allowed) = property.get("enum") {
                assert!(
                    allowed
                        .as_array()
                        .expect("enum is an array")
                        .iter()
                        .any(|candidate| candidate == value),
                    "{path} value {value} is outside the pinned enum"
                );
            }
        }
        "integer" => assert!(value.is_i64(), "{path} must be an integer"),
        "boolean" => assert!(value.is_boolean(), "{path} must be a boolean"),
        "array" => {
            for (index, item) in value
                .as_array()
                .unwrap_or_else(|| panic!("{path} must be an array"))
                .iter()
                .enumerate()
            {
                assert_envelope_value(types, &property["items"], item, &format!("{path}[{index}]"));
            }
        }
        type_name => assert_envelope_type(types, type_name, value, path),
    }
    if let Some(constant) = property.get("const") {
        assert_eq!(value, constant, "{path} is pinned to {constant}");
    }
}

#[test]
fn handoff_envelope_v1_matches_the_frozen_manifest() {
    use contractorcrm_lib::application::{
        create_company, create_contact, create_opportunity, export_handoff_envelope, link_job,
        link_quote, list_stages, move_opportunity_stage, ChannelInput, CompanyPatch, ContactPatch,
        CreateCompanyRequest, CreateContactRequest, CreateOpportunityRequest, HandoffRefInput,
        LinkJobRequest, LinkQuoteRequest, MoveOpportunityStageRequest, OpportunityPatch,
        HANDOFF_SCHEMA_VERSION,
    };
    use contractorcrm_lib::domain::{Actor, StageKind};

    let schema: Value =
        serde_json::from_str(HANDOFF_ENVELOPE_SCHEMA).expect("valid hand-off envelope schema");
    assert_eq!(schema["contract"], "contractorcrm-handoff-envelope");
    assert_eq!(schema["envelopeVersion"], HANDOFF_SCHEMA_VERSION);
    assert_eq!(schema["root"], "HandoffEnvelope");
    let types = &schema["types"];

    let temp = tempfile::tempdir().expect("create temporary app data");
    let mut storage = storage::Storage::open_in_app_data(temp.path()).expect("open storage");

    // A fully populated opportunity so every non-null arm of the manifest is
    // exercised: company, contact with channels, money, refs, stage name.
    let company = create_company(
        &mut storage,
        CreateCompanyRequest {
            actor: Actor::User,
            company: CompanyPatch {
                name: "Ruiz Property Group".into(),
                kind: "client".into(),
                phone: Some("555-0142".into()),
                email: Some("office@example.com".into()),
                website: Some("https://example.com".into()),
                address_line1: Some("120 Palm Ave".into()),
                address_line2: Some("Suite 3".into()),
                city: Some("Orlando".into()),
                state: Some("FL".into()),
                postal_code: Some("32801".into()),
                service_area: Some("Central Florida".into()),
                license_notes: Some("CGC1500000".into()),
                notes: Some("Repeat client".into()),
            },
        },
    )
    .expect("create company");
    let contact = create_contact(
        &mut storage,
        CreateContactRequest {
            actor: Actor::User,
            contact: ContactPatch {
                company_id: Some(company.id.clone()),
                first_name: Some("Dana".into()),
                last_name: Some("Ruiz".into()),
                display_name: Some("Dana Ruiz".into()),
                role: Some("owner".into()),
                kind: "client".into(),
                preferred_contact_method: Some("phone".into()),
                address_line1: Some("120 Palm Ave".into()),
                address_line2: Some("Suite 3".into()),
                city: Some("Orlando".into()),
                state: Some("FL".into()),
                postal_code: Some("32801".into()),
                property_type: Some("residential".into()),
                notes: Some("Prefers mornings".into()),
                favorite: true,
                channels: vec![ChannelInput {
                    kind: "phone".into(),
                    label: Some("mobile".into()),
                    value: "555-0100".into(),
                    preferred: true,
                }],
            },
        },
    )
    .expect("create contact");
    let opportunity = create_opportunity(
        &mut storage,
        CreateOpportunityRequest {
            actor: Actor::User,
            stage_id: None,
            opportunity: OpportunityPatch {
                name: "Backyard privacy fence".into(),
                contact_id: Some(contact.id.clone()),
                company_id: Some(company.id.clone()),
                value_minor: 250_000,
                currency_code: "USD".into(),
                probability_percent: Some(80),
                expected_close_date: Some("2026-09-01".into()),
                source: Some("referral".into()),
                source_label: Some("Neighbor on Palm Ave".into()),
                notes: Some("Six-foot board on board".into()),
            },
        },
    )
    .expect("create opportunity");
    let won_stage = list_stages(&storage)
        .expect("list stages")
        .into_iter()
        .find(|stage| stage.kind == StageKind::Won)
        .expect("won stage seeded");
    let opportunity = move_opportunity_stage(
        &mut storage,
        MoveOpportunityStageRequest {
            actor: Actor::User,
            opportunity_id: opportunity.id.clone(),
            to_stage_id: won_stage.id,
            lost_reason_id: None,
            expected_version: opportunity.version,
        },
    )
    .expect("move to won");
    let opportunity = link_quote(
        &mut storage,
        LinkQuoteRequest {
            actor: Actor::User,
            opportunity_id: opportunity.id.clone(),
            expected_version: opportunity.version,
            quote_ref: HandoffRefInput {
                tool: "quoter".into(),
                external_id: "123".into(),
                label: Some("Q-123".into()),
            },
        },
    )
    .expect("link quote");
    let opportunity = link_job(
        &mut storage,
        LinkJobRequest {
            actor: Actor::User,
            opportunity_id: opportunity.id.clone(),
            expected_version: opportunity.version,
            job_ref: HandoffRefInput {
                tool: "contractorproject".into(),
                external_id: "job-1".into(),
                label: Some("Backyard privacy fence".into()),
            },
        },
    )
    .expect("link job");

    let destination = temp.path().join("full-handoff.json");
    let report = export_handoff_envelope(
        &mut storage,
        &opportunity.id,
        destination.to_str().expect("utf-8 path"),
        false,
    )
    .expect("export the populated envelope");
    assert_eq!(report.schema_version, HANDOFF_SCHEMA_VERSION);
    let populated: Value =
        serde_json::from_str(&std::fs::read_to_string(&destination).expect("read envelope"))
            .expect("valid envelope JSON");
    assert_envelope_type(types, "HandoffEnvelope", &populated, "envelope");
    // The populated export really does fill the optional arms.
    assert_eq!(populated["opportunity"]["stageName"], "Won");
    assert_eq!(populated["opportunity"]["quoteRef"]["tool"], "quoter");
    assert_eq!(
        populated["opportunity"]["jobRef"]["tool"],
        "contractorproject"
    );
    assert!(populated["contact"]["channels"][0].is_object());
    assert_eq!(populated["company"]["name"], "Ruiz Property Group");

    // A bare opportunity covers the null arms. Every opportunity needs a
    // contact or a company, so the contact-only export nulls the company and
    // the company-only export nulls the contact.
    let bare_contact = create_contact(
        &mut storage,
        CreateContactRequest {
            actor: Actor::User,
            contact: ContactPatch {
                display_name: Some("Walk-in".into()),
                kind: "lead".into(),
                ..ContactPatch::default()
            },
        },
    )
    .expect("create bare contact");
    let minimal = create_opportunity(
        &mut storage,
        CreateOpportunityRequest {
            actor: Actor::User,
            stage_id: None,
            opportunity: OpportunityPatch {
                name: "Gate repair".into(),
                contact_id: Some(bare_contact.id.clone()),
                company_id: None,
                value_minor: 0,
                currency_code: "USD".into(),
                probability_percent: None,
                expected_close_date: None,
                source: None,
                source_label: None,
                notes: None,
            },
        },
    )
    .expect("create minimal opportunity");
    let minimal_destination = temp.path().join("minimal-handoff.json");
    export_handoff_envelope(
        &mut storage,
        &minimal.id,
        minimal_destination.to_str().expect("utf-8 path"),
        false,
    )
    .expect("export the minimal envelope");
    let bare: Value = serde_json::from_str(
        &std::fs::read_to_string(&minimal_destination).expect("read envelope"),
    )
    .expect("valid envelope JSON");
    assert_envelope_type(types, "HandoffEnvelope", &bare, "envelope");
    assert!(bare["company"].is_null());
    assert!(bare["opportunity"]["quoteRef"].is_null());
    assert!(bare["opportunity"]["jobRef"].is_null());
    assert!(bare["opportunity"]["source"].is_null());
    assert!(bare["opportunity"]["expectedCloseDate"].is_null());

    // Company-only work orders leave the contact null.
    let company_only = create_opportunity(
        &mut storage,
        CreateOpportunityRequest {
            actor: Actor::User,
            stage_id: None,
            opportunity: OpportunityPatch {
                name: "Warehouse chain link".into(),
                contact_id: None,
                company_id: Some(company.id.clone()),
                value_minor: 0,
                currency_code: "USD".into(),
                probability_percent: None,
                expected_close_date: None,
                source: None,
                source_label: None,
                notes: None,
            },
        },
    )
    .expect("create company-only opportunity");
    let company_only_destination = temp.path().join("company-only-handoff.json");
    export_handoff_envelope(
        &mut storage,
        &company_only.id,
        company_only_destination.to_str().expect("utf-8 path"),
        false,
    )
    .expect("export the company-only envelope");
    let without_contact: Value = serde_json::from_str(
        &std::fs::read_to_string(&company_only_destination).expect("read envelope"),
    )
    .expect("valid envelope JSON");
    assert_envelope_type(types, "HandoffEnvelope", &without_contact, "envelope");
    assert!(without_contact["contact"].is_null());

    // An undocumented field is a contract change, not a free addition.
    let mut with_extra = populated.clone();
    with_extra["opportunity"]["timezone"] = Value::String("America/New_York".into());
    assert!(
        std::panic::catch_unwind(|| assert_envelope_type(
            types,
            "HandoffEnvelope",
            &with_extra,
            "envelope"
        ))
        .is_err(),
        "an unpinned envelope field must fail the contract test"
    );
    // So is dropping a pinned one.
    let mut missing_field = populated.clone();
    missing_field["opportunity"]
        .as_object_mut()
        .expect("opportunity object")
        .remove("stageName");
    assert!(
        std::panic::catch_unwind(|| assert_envelope_type(
            types,
            "HandoffEnvelope",
            &missing_field,
            "envelope"
        ))
        .is_err(),
        "a removed envelope field must fail the contract test"
    );
}
