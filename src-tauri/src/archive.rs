//! Portable archive export/import (docs/DATA_MODEL.md "Archive contract").
//!
//! An archive is a versioned ZIP holding `manifest.json`, one JSON file per
//! canonical table under `data/`, human-readable CSV copies under `csv/`, and a
//! confined `assets/` directory reserved for attachments. Import verifies the
//! whole file — paths, checksums, versions, row shapes, and referential
//! integrity — before the live database is touched, then replaces every
//! canonical row in a single transaction.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};

use rusqlite::types::{Value, ValueRef};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::application::{
    check_export_destination, immediate, log_command, rebuild_search_index, write_contacts_csv,
    write_opportunities_csv, ProductInfo,
};
use crate::domain::Actor;
use crate::error::ApplicationError;
use crate::storage::{self, now_utc, Storage};

/// Archive schema version — additive changes only within a major version;
/// breaking changes bump this number.
pub const ARCHIVE_SCHEMA_VERSION: i64 = 1;

/// Canonical tables carried by an archive, in dependency order: inserting in
/// this order and deleting in reverse never trips a foreign key or a
/// polymorphic owner trigger. `command_log`, `app_settings`, `search_index`,
/// and `schema_migrations` are deliberately excluded — the log and settings are
/// local history, the index is rebuilt, and migrations belong to the database.
const ARCHIVE_TABLES: &[&str] = &[
    "companies",
    "contacts",
    "contact_channels",
    "pipelines",
    "stages",
    "lost_reasons",
    "opportunities",
    "stage_history",
    "activities",
    "tasks",
    "saved_views",
    "tags",
    "record_tags",
    "custom_field_defs",
    "custom_field_options",
    "custom_field_values",
];

/// Plain foreign keys checked in memory before any write: (table, column,
/// referenced table). Null values are allowed wherever the column is nullable.
const ARCHIVE_FOREIGN_KEYS: &[(&str, &str, &str)] = &[
    ("contacts", "company_id", "companies"),
    ("contact_channels", "contact_id", "contacts"),
    ("stages", "pipeline_id", "pipelines"),
    ("opportunities", "contact_id", "contacts"),
    ("opportunities", "company_id", "companies"),
    ("opportunities", "stage_id", "stages"),
    ("opportunities", "lost_reason_id", "lost_reasons"),
    ("stage_history", "opportunity_id", "opportunities"),
    ("stage_history", "from_stage_id", "stages"),
    ("stage_history", "to_stage_id", "stages"),
    ("stage_history", "lost_reason_id", "lost_reasons"),
    ("record_tags", "tag_id", "tags"),
    ("custom_field_options", "definition_id", "custom_field_defs"),
    ("custom_field_values", "definition_id", "custom_field_defs"),
    ("custom_field_values", "option_id", "custom_field_options"),
];

/// Polymorphic parents checked in memory: (table, type column, id column).
/// SQLite cannot express these as foreign keys, so the archive validates them
/// the way the application layer does for interactive writes.
const ARCHIVE_POLYMORPHIC_KEYS: &[(&str, &str, &str)] = &[
    ("activities", "parent_type", "parent_id"),
    ("tasks", "parent_type", "parent_id"),
    ("record_tags", "entity_type", "record_id"),
    ("custom_field_values", "entity_type", "record_id"),
];

/// Table each polymorphic `*_type` value points at.
fn polymorphic_table(parent_type: &str) -> Option<&'static str> {
    match parent_type {
        "contact" => Some("contacts"),
        "company" => Some("companies"),
        "opportunity" => Some("opportunities"),
        _ => None,
    }
}

/// Primary key columns used for duplicate detection during verification.
fn primary_key_columns(table: &str) -> &'static [&'static str] {
    match table {
        "record_tags" => &["tag_id", "entity_type", "record_id"],
        _ => &["id"],
    }
}

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

/// One archived file with the checksum import verifies it against.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArchiveFileEntry {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

/// `manifest.json` at the root of every archive.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArchiveManifest {
    pub schema_version: i64,
    pub product: ProductInfo,
    pub exported_at: String,
    pub database_migration_version: i64,
    pub files: Vec<ArchiveFileEntry>,
    pub record_counts: BTreeMap<String, i64>,
}

/// Where an archive landed and what it holds.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArchiveExportReport {
    pub path: String,
    pub record_counts: BTreeMap<String, i64>,
    pub file_count: usize,
}

/// A machine-readable reason an archive cannot be imported.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArchiveIssue {
    pub code: String,
    pub message: String,
}

/// Read-only verification result: what the archive claims plus every problem
/// found. An empty `issues` list means `import_archive` would succeed.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArchiveImportPreview {
    pub schema_version: i64,
    pub product: ProductInfo,
    pub exported_at: String,
    pub database_migration_version: i64,
    pub record_counts: BTreeMap<String, i64>,
    pub issues: Vec<ArchiveIssue>,
}

/// What an import wrote and where the pre-import safety backup went.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArchiveImportReport {
    pub record_counts: BTreeMap<String, i64>,
    pub safety_backup_path: String,
}

// ---------------------------------------------------------------------------
// Column specs
// ---------------------------------------------------------------------------

/// SQLite storage classes the archive knows how to round-trip.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ColumnKind {
    Text,
    Integer,
    Real,
}

/// One canonical column: database name, its camelCase JSON key, and the shape
/// an archived value must have.
struct ColumnSpec {
    name: String,
    camel: String,
    kind: ColumnKind,
    not_null: bool,
}

/// snake_case column name to the camelCase key used in archive JSON.
fn camel_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut upper_next = false;
    for character in name.chars() {
        if character == '_' {
            upper_next = true;
            continue;
        }
        if upper_next {
            out.extend(character.to_uppercase());
            upper_next = false;
        } else {
            out.push(character);
        }
    }
    out
}

/// Column specs for a canonical table, read from the live schema so the
/// archive can never drift from the migrations.
fn table_columns(
    connection: &Connection,
    table: &str,
) -> Result<Vec<ColumnSpec>, ApplicationError> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info(\"{table}\")"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)? == 1,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(name, declared, not_null)| {
            let kind = match declared.to_ascii_uppercase().as_str() {
                "TEXT" => ColumnKind::Text,
                "INTEGER" => ColumnKind::Integer,
                "REAL" => ColumnKind::Real,
                other => {
                    return Err(ApplicationError::InvalidStoredData(format!(
                        "{table}.{name} has unsupported column type {other}"
                    )))
                }
            };
            Ok(ColumnSpec {
                camel: camel_case(&name),
                name,
                kind,
                not_null,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

/// Write a portable archive of every canonical table to `path`. Export is
/// user-initiated, so the command log actor is always `user`.
pub fn export_archive(
    storage: &mut Storage,
    path: &str,
    overwrite: bool,
) -> Result<ArchiveExportReport, ApplicationError> {
    let path = check_export_destination(path, overwrite)?;
    let connection = storage.connection();

    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut record_counts = BTreeMap::new();
    for table in ARCHIVE_TABLES {
        let (rows, count) = export_table(connection, table)?;
        record_counts.insert((*table).to_owned(), count);
        files.push((format!("data/{table}.json"), rows));
    }

    // Human-readable convenience copies; import ignores them.
    let mut contacts_csv = csv::Writer::from_writer(Vec::new());
    write_contacts_csv(connection, &mut contacts_csv)?;
    files.push(("csv/contacts.csv".to_owned(), csv_bytes(contacts_csv)?));
    let mut opportunities_csv = csv::Writer::from_writer(Vec::new());
    write_opportunities_csv(connection, &mut opportunities_csv)?;
    files.push((
        "csv/opportunities.csv".to_owned(),
        csv_bytes(opportunities_csv)?,
    ));

    let manifest = ArchiveManifest {
        schema_version: ARCHIVE_SCHEMA_VERSION,
        product: ProductInfo {
            name: "ContractorCRM".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
        exported_at: now_utc(),
        database_migration_version: storage::latest_migration_version(),
        files: files
            .iter()
            .map(|(name, bytes)| ArchiveFileEntry {
                path: name.clone(),
                sha256: sha256_hex(bytes),
                bytes: bytes.len() as u64,
            })
            .collect(),
        record_counts: record_counts.clone(),
    };
    let manifest_json = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| ApplicationError::InvalidStoredData(error.to_string()))?;

    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    writer
        .start_file("manifest.json", options)
        .map_err(zip_write_error)?;
    writer.write_all(&manifest_json)?;
    for (name, bytes) in &files {
        writer.start_file(name, options).map_err(zip_write_error)?;
        writer.write_all(bytes)?;
    }
    // Reserved for attachments (issue #21); empty for now.
    writer
        .add_directory("assets", options)
        .map_err(zip_write_error)?;
    let archive_bytes = writer.finish().map_err(zip_write_error)?.into_inner();

    if let Some(parent) = std::path::Path::new(&path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(&path, archive_bytes)?;

    let file_count = files.len() + 1;
    let total: i64 = record_counts.values().sum();
    let transaction = immediate(storage)?;
    log_command(
        &transaction,
        Actor::User,
        "export",
        "archive",
        &format!("exported archive with {total} records to \"{path}\""),
    )?;
    transaction.commit()?;

    Ok(ArchiveExportReport {
        path,
        record_counts,
        file_count,
    })
}

/// Serialize one canonical table as a pretty JSON array of camelCase rows.
fn export_table(connection: &Connection, table: &str) -> Result<(Vec<u8>, i64), ApplicationError> {
    let columns = table_columns(connection, table)?;
    let selection = columns
        .iter()
        .map(|column| format!("\"{}\"", column.name))
        .collect::<Vec<_>>()
        .join(", ");
    let mut statement = connection.prepare(&format!(
        "SELECT {selection} FROM \"{table}\" ORDER BY rowid"
    ))?;
    let rows = statement
        .query_map([], |row| {
            let mut object = serde_json::Map::with_capacity(columns.len());
            for (index, column) in columns.iter().enumerate() {
                object.insert(column.camel.clone(), value_to_json(row.get_ref(index)?)?);
            }
            Ok(serde_json::Value::Object(object))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let count = rows.len() as i64;
    let json = serde_json::to_vec_pretty(&serde_json::Value::Array(rows))
        .map_err(|error| ApplicationError::InvalidStoredData(error.to_string()))?;
    Ok((json, count))
}

/// Stored value to archive JSON. Blobs never appear in the CRM schema, so one
/// showing up means the database is not ours.
fn value_to_json(value: ValueRef<'_>) -> rusqlite::Result<serde_json::Value> {
    Ok(match value {
        ValueRef::Null => serde_json::Value::Null,
        ValueRef::Integer(number) => serde_json::Value::from(number),
        ValueRef::Real(number) => serde_json::Value::from(number),
        ValueRef::Text(_) => serde_json::Value::from(value.as_str()?.to_owned()),
        ValueRef::Blob(_) => {
            return Err(rusqlite::Error::InvalidQuery);
        }
    })
}

fn csv_bytes(writer: csv::Writer<Vec<u8>>) -> Result<Vec<u8>, ApplicationError> {
    writer
        .into_inner()
        .map_err(|error| ApplicationError::Io(std::io::Error::other(error.to_string())))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn zip_write_error(error: zip::result::ZipError) -> ApplicationError {
    ApplicationError::Io(std::io::Error::other(error.to_string()))
}

// ---------------------------------------------------------------------------
// Verification
// ---------------------------------------------------------------------------

/// One validated row: values in canonical column order.
struct ArchiveRow {
    values: Vec<Value>,
}

/// A fully verified archive: the manifest, the rows it carries, and every
/// problem found. Rows are only trustworthy when `issues` is empty.
struct VerifiedArchive {
    manifest: ArchiveManifest,
    tables: BTreeMap<&'static str, Vec<ArchiveRow>>,
    issues: Vec<ArchiveIssue>,
}

fn issue(code: &str, message: String) -> ArchiveIssue {
    ArchiveIssue {
        code: code.to_owned(),
        message,
    }
}

/// Unreadable containers are caller errors — a missing file or a file that is
/// not an archive at all cannot be reported per-record.
fn unreadable(path: &str, message: String) -> ApplicationError {
    ApplicationError::InvalidInput {
        field: "path".into(),
        message: format!("cannot read archive \"{path}\": {message}"),
    }
}

/// Verify an archive end to end without writing anything.
fn verify_archive(
    connection: &Connection,
    path: &str,
) -> Result<VerifiedArchive, ApplicationError> {
    let bytes = std::fs::read(path).map_err(|error| unreadable(path, error.to_string()))?;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes))
        .map_err(|error| unreadable(path, error.to_string()))?;

    let mut issues = Vec::new();
    let mut entries: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|error| unreadable(path, error.to_string()))?;
        let name = entry.name().to_owned();
        if let Some(problem) = entry_path_issue(&name) {
            issues.push(problem);
            continue;
        }
        if entry.is_dir() {
            continue;
        }
        let mut content = Vec::with_capacity(entry.size() as usize);
        entry
            .read_to_end(&mut content)
            .map_err(|error| unreadable(path, error.to_string()))?;
        entries.insert(name, content);
    }
    // Duplicate names collapse when the central directory is indexed, so the
    // declared entry count is what catches a smuggled second copy.
    if let Some(declared) = declared_entry_count(&bytes) {
        if declared as usize != zip.len() {
            issues.push(issue(
                "duplicate_entry",
                format!(
                    "archive declares {declared} entries but only {} are unique",
                    zip.len()
                ),
            ));
        }
    }

    let manifest_bytes = entries
        .get("manifest.json")
        .ok_or_else(|| unreadable(path, "no manifest.json".into()))?;
    let manifest: ArchiveManifest = serde_json::from_slice(manifest_bytes)
        .map_err(|error| unreadable(path, format!("manifest.json is invalid: {error}")))?;

    verify_manifest_versions(&manifest, &mut issues);
    verify_checksums(&manifest, &entries, &mut issues);

    let mut tables = BTreeMap::new();
    for table in ARCHIVE_TABLES {
        let columns = table_columns(connection, table)?;
        let file = format!("data/{table}.json");
        let Some(content) = entries.get(&file) else {
            issues.push(issue(
                "missing_table_file",
                format!("archive has no {file}"),
            ));
            continue;
        };
        match parse_table(table, &columns, content, &mut issues) {
            Some(rows) => {
                tables.insert(*table, rows);
            }
            None => continue,
        }
    }
    if issues.is_empty() {
        verify_references(connection, &tables, &mut issues)?;
    }

    Ok(VerifiedArchive {
        manifest,
        tables,
        issues,
    })
}

/// Reject anything that could escape the archive root; `assets/` and `csv/`
/// are carried but ignored, everything else must be a known data file.
fn entry_path_issue(name: &str) -> Option<ArchiveIssue> {
    if name.contains('\\') {
        return Some(issue(
            "entry_path_backslash",
            format!("entry \"{name}\" uses backslashes"),
        ));
    }
    if name.starts_with('/') || name.chars().nth(1) == Some(':') {
        return Some(issue(
            "entry_path_absolute",
            format!("entry \"{name}\" is an absolute path"),
        ));
    }
    if name
        .split('/')
        .any(|component| component == ".." || component == ".")
    {
        return Some(issue(
            "entry_path_traversal",
            format!("entry \"{name}\" escapes the archive root"),
        ));
    }
    let directory = name.ends_with('/');
    let known = if directory {
        matches!(name, "data/" | "csv/" | "assets/")
    } else {
        name == "manifest.json"
            || name.starts_with("csv/")
            || name.starts_with("assets/")
            || ARCHIVE_TABLES
                .iter()
                .any(|table| name == format!("data/{table}.json"))
    };
    if !known {
        return Some(issue(
            "unknown_file",
            format!("entry \"{name}\" is not part of the archive contract"),
        ));
    }
    None
}

/// Number of central directory records the archive claims, read from the end
/// of central directory record. `None` when it cannot be read (ZIP64).
fn declared_entry_count(bytes: &[u8]) -> Option<u16> {
    let signature = [0x50, 0x4b, 0x05, 0x06];
    let start = bytes.len().saturating_sub(u16::MAX as usize + 22);
    let offset = bytes[start..]
        .windows(4)
        .rposition(|window| window == signature)?
        + start;
    let count = u16::from_le_bytes([*bytes.get(offset + 10)?, *bytes.get(offset + 11)?]);
    (count != u16::MAX).then_some(count)
}

fn verify_manifest_versions(manifest: &ArchiveManifest, issues: &mut Vec<ArchiveIssue>) {
    if manifest.schema_version != ARCHIVE_SCHEMA_VERSION {
        issues.push(issue(
            "unsupported_schema_version",
            format!(
                "archive schema version {} is not supported (expected {ARCHIVE_SCHEMA_VERSION})",
                manifest.schema_version
            ),
        ));
    }
    let supported = storage::latest_migration_version();
    if manifest.database_migration_version > supported {
        issues.push(issue(
            "unsupported_migration_version",
            format!(
                "archive was written at database version {} but this app supports {supported}; \
                 update the app first",
                manifest.database_migration_version
            ),
        ));
    }
}

/// Every listed file must be present and match its checksum, and every data
/// file present must be listed.
fn verify_checksums(
    manifest: &ArchiveManifest,
    entries: &BTreeMap<String, Vec<u8>>,
    issues: &mut Vec<ArchiveIssue>,
) {
    let mut listed = BTreeSet::new();
    for file in &manifest.files {
        listed.insert(file.path.clone());
        let Some(content) = entries.get(&file.path) else {
            issues.push(issue(
                "missing_file",
                format!(
                    "manifest lists \"{}\" but the archive has no such file",
                    file.path
                ),
            ));
            continue;
        };
        if content.len() as u64 != file.bytes {
            issues.push(issue(
                "size_mismatch",
                format!(
                    "\"{}\" is {} bytes but the manifest says {}",
                    file.path,
                    content.len(),
                    file.bytes
                ),
            ));
        }
        if sha256_hex(content) != file.sha256 {
            issues.push(issue(
                "checksum_mismatch",
                format!("\"{}\" does not match its manifest checksum", file.path),
            ));
        }
    }
    for name in entries.keys() {
        if name == "manifest.json" || name.starts_with("assets/") || listed.contains(name) {
            continue;
        }
        issues.push(issue(
            "unlisted_file",
            format!("\"{name}\" is in the archive but not listed in the manifest"),
        ));
    }
}

/// Parse and shape-check one table file. Returns None when the file itself is
/// unusable, so callers stop looking at that table.
fn parse_table(
    table: &str,
    columns: &[ColumnSpec],
    content: &[u8],
    issues: &mut Vec<ArchiveIssue>,
) -> Option<Vec<ArchiveRow>> {
    let parsed: serde_json::Value = match serde_json::from_slice(content) {
        Ok(value) => value,
        Err(error) => {
            issues.push(issue(
                "invalid_table_json",
                format!("data/{table}.json is not valid JSON: {error}"),
            ));
            return None;
        }
    };
    let Some(array) = parsed.as_array() else {
        issues.push(issue(
            "invalid_table_json",
            format!("data/{table}.json is not a JSON array"),
        ));
        return None;
    };

    let mut rows = Vec::with_capacity(array.len());
    let mut seen_keys = BTreeSet::new();
    for (index, item) in array.iter().enumerate() {
        let Some(object) = item.as_object() else {
            issues.push(issue(
                "invalid_table_json",
                format!("{table} row {index} is not a JSON object"),
            ));
            continue;
        };
        for key in object.keys() {
            if !columns.iter().any(|column| &column.camel == key) {
                issues.push(issue(
                    "unknown_column",
                    format!("{table} row {index} has unknown field \"{key}\""),
                ));
            }
        }
        let mut values = Vec::with_capacity(columns.len());
        let mut row_valid = true;
        for column in columns {
            match archive_value(table, index, column, object.get(&column.camel)) {
                Ok(value) => values.push(value),
                Err(problem) => {
                    issues.push(problem);
                    row_valid = false;
                }
            }
        }
        if !row_valid {
            continue;
        }
        // Ids identify records across databases; blank ones cannot.
        for (position, column) in columns.iter().enumerate() {
            if column.name == "id" || column.name.ends_with("_id") {
                if let Value::Text(text) = &values[position] {
                    if text.trim().is_empty() {
                        issues.push(issue(
                            "invalid_id",
                            format!("{table} row {index} has an empty {}", column.camel),
                        ));
                        row_valid = false;
                    }
                }
            }
            if column.name == "version" {
                if let Value::Integer(number) = &values[position] {
                    if *number < 1 {
                        issues.push(issue(
                            "invalid_version",
                            format!("{table} row {index} has version {number}"),
                        ));
                        row_valid = false;
                    }
                }
            }
        }
        if !row_valid {
            continue;
        }
        let key = primary_key_columns(table)
            .iter()
            .map(|name| {
                let position = columns
                    .iter()
                    .position(|column| &column.name == name)
                    .expect("primary key column exists");
                match &values[position] {
                    Value::Text(text) => text.clone(),
                    other => format!("{other:?}"),
                }
            })
            .collect::<Vec<_>>()
            .join("\u{1f}");
        if !seen_keys.insert(key.clone()) {
            issues.push(issue(
                "duplicate_primary_key",
                format!("{table} has more than one row with key \"{key}\""),
            ));
            continue;
        }
        rows.push(ArchiveRow { values });
    }
    Some(rows)
}

/// One archived cell to its stored value. Missing keys are allowed only for
/// nullable columns, which is how archives from older schema versions import.
fn archive_value(
    table: &str,
    index: usize,
    column: &ColumnSpec,
    value: Option<&serde_json::Value>,
) -> Result<Value, ArchiveIssue> {
    let invalid = |detail: &str| {
        issue(
            "invalid_value",
            format!("{table} row {index} field \"{}\" {detail}", column.camel),
        )
    };
    let Some(value) = value else {
        if column.not_null {
            return Err(issue(
                "missing_column",
                format!("{table} row {index} is missing \"{}\"", column.camel),
            ));
        }
        return Ok(Value::Null);
    };
    match value {
        serde_json::Value::Null if column.not_null => Err(invalid("must not be null")),
        serde_json::Value::Null => Ok(Value::Null),
        serde_json::Value::String(text) if column.kind == ColumnKind::Text => {
            Ok(Value::Text(text.clone()))
        }
        serde_json::Value::Number(number) if column.kind == ColumnKind::Integer => number
            .as_i64()
            .map(Value::Integer)
            .ok_or_else(|| invalid("must be a whole number")),
        serde_json::Value::Number(number) if column.kind == ColumnKind::Real => number
            .as_f64()
            .map(Value::Real)
            .ok_or_else(|| invalid("must be a number")),
        _ => Err(invalid(match column.kind {
            ColumnKind::Text => "must be text",
            ColumnKind::Integer => "must be a whole number",
            ColumnKind::Real => "must be a number",
        })),
    }
}

/// Validate every relationship among the archived tables in memory, so a
/// dangling reference is reported instead of failing mid-write.
fn verify_references(
    connection: &Connection,
    tables: &BTreeMap<&'static str, Vec<ArchiveRow>>,
    issues: &mut Vec<ArchiveIssue>,
) -> Result<(), ApplicationError> {
    let mut columns = BTreeMap::new();
    let mut ids: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    for table in ARCHIVE_TABLES {
        let specs = table_columns(connection, table)?;
        if let (Some(rows), Some(position)) = (
            tables.get(table),
            specs.iter().position(|column| column.name == "id"),
        ) {
            let mut set = BTreeSet::new();
            for row in rows {
                if let Value::Text(text) = &row.values[position] {
                    set.insert(text.clone());
                }
            }
            ids.insert(*table, set);
        }
        columns.insert(*table, specs);
    }

    let cell = |table: &str, row: &ArchiveRow, column: &str| -> Option<String> {
        let position = columns
            .get(table)?
            .iter()
            .position(|spec| spec.name == column)?;
        match &row.values[position] {
            Value::Text(text) => Some(text.clone()),
            _ => None,
        }
    };

    for (table, column, target) in ARCHIVE_FOREIGN_KEYS {
        let Some(rows) = tables.get(table) else {
            continue;
        };
        for (index, row) in rows.iter().enumerate() {
            let Some(value) = cell(table, row, column) else {
                continue; // NULL references are allowed where the column is nullable
            };
            let known = ids.get(target).is_some_and(|set| set.contains(&value));
            if !known {
                issues.push(issue(
                    "missing_reference",
                    format!(
                        "{table} row {index} references {target} \"{value}\", which the archive \
                         does not contain"
                    ),
                ));
            }
        }
    }

    for (table, type_column, id_column) in ARCHIVE_POLYMORPHIC_KEYS {
        let Some(rows) = tables.get(table) else {
            continue;
        };
        for (index, row) in rows.iter().enumerate() {
            let (Some(parent_type), Some(parent_id)) =
                (cell(table, row, type_column), cell(table, row, id_column))
            else {
                continue; // both null together, enforced by the table CHECK
            };
            let Some(target) = polymorphic_table(&parent_type) else {
                issues.push(issue(
                    "unknown_parent_type",
                    format!("{table} row {index} has unknown {type_column} \"{parent_type}\""),
                ));
                continue;
            };
            if !ids.get(target).is_some_and(|set| set.contains(&parent_id)) {
                issues.push(issue(
                    "missing_reference",
                    format!(
                        "{table} row {index} references {target} \"{parent_id}\", which the \
                         archive does not contain"
                    ),
                ));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Preview and import
// ---------------------------------------------------------------------------

/// Verify an archive and report what it holds. Never writes to the database.
pub fn preview_archive_import(
    storage: &Storage,
    path: &str,
) -> Result<ArchiveImportPreview, ApplicationError> {
    let verified = verify_archive(storage.connection(), path)?;
    Ok(ArchiveImportPreview {
        schema_version: verified.manifest.schema_version,
        product: verified.manifest.product.clone(),
        exported_at: verified.manifest.exported_at.clone(),
        database_migration_version: verified.manifest.database_migration_version,
        record_counts: verified.manifest.record_counts.clone(),
        issues: verified.issues,
    })
}

/// Replace every canonical record with the archive's, after full verification
/// and a timestamped safety backup. The delete/insert/reindex happens in one
/// transaction, so a failure leaves the live database exactly as it was.
pub fn import_archive(
    storage: &mut Storage,
    path: &str,
) -> Result<ArchiveImportReport, ApplicationError> {
    let verified = verify_archive(storage.connection(), path)?;
    if let Some(first) = verified.issues.first() {
        let extra = verified.issues.len() - 1;
        let suffix = match extra {
            0 => String::new(),
            1 => " (and 1 more problem)".to_owned(),
            more => format!(" (and {more} more problems)"),
        };
        return Err(ApplicationError::ValidationFailed {
            code: "archive_invalid",
            field: "path".into(),
            message: format!("{}{suffix}", first.message),
        });
    }

    let mut specs = BTreeMap::new();
    for table in ARCHIVE_TABLES {
        specs.insert(*table, table_columns(storage.connection(), table)?);
    }
    let safety_backup_path = storage.safety_copy("pre-import")?;

    let mut record_counts = BTreeMap::new();
    let transaction = immediate(storage)?;
    for table in ARCHIVE_TABLES.iter().rev() {
        transaction.execute(&format!("DELETE FROM \"{table}\""), [])?;
    }
    for table in ARCHIVE_TABLES {
        let columns = &specs[table];
        let rows = verified.tables.get(table).map(Vec::as_slice).unwrap_or(&[]);
        let names = columns
            .iter()
            .map(|column| format!("\"{}\"", column.name))
            .collect::<Vec<_>>()
            .join(", ");
        let placeholders = (1..=columns.len())
            .map(|position| format!("?{position}"))
            .collect::<Vec<_>>()
            .join(", ");
        let mut statement = transaction.prepare(&format!(
            "INSERT INTO \"{table}\" ({names}) VALUES ({placeholders})"
        ))?;
        for row in rows {
            statement.execute(rusqlite::params_from_iter(row.values.iter()))?;
        }
        drop(statement);
        record_counts.insert((*table).to_owned(), rows.len() as i64);
    }
    rebuild_search_index(&transaction)?;
    let total: i64 = record_counts.values().sum();
    log_command(
        &transaction,
        Actor::User,
        "import",
        "archive",
        &format!(
            "imported archive \"{path}\" with {total} records; safety backup at \"{}\"",
            safety_backup_path.display()
        ),
    )?;
    transaction.commit()?;

    Ok(ArchiveImportReport {
        record_counts,
        safety_backup_path: safety_backup_path.to_string_lossy().into_owned(),
    })
}
