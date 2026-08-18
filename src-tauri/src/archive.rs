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
    check_export_destination, csv_bytes, immediate, log_command, rebuild_search_index,
    write_contacts_csv, write_export_file, write_opportunities_csv, ProductInfo,
};
use crate::attachments::{AttachmentStore, IMPORT_STAGING_PREFIX};
use crate::domain::Actor;
use crate::error::ApplicationError;
use crate::storage::{self, now_utc, Storage};

/// Archive schema version — additive changes only within a major version;
/// breaking changes bump this number.
pub const ARCHIVE_SCHEMA_VERSION: i64 = 1;

/// Directory inside an archive holding managed attachment files, one
/// `assets/<attachment id>/<file name>` entry per attachments row.
const ASSET_PREFIX: &str = "assets/";

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
    "attachments",
];

/// Migration that first created each canonical table. An archive written
/// before a table existed cannot carry its data file, so a missing file is
/// tolerated as "no rows" when the archive predates the table — and is still
/// an issue for any archive that should have written it.
const TABLE_INTRODUCED_IN: &[(&str, i64)] = &[
    ("companies", 1),
    ("contacts", 1),
    ("contact_channels", 1),
    ("pipelines", 2),
    ("stages", 2),
    ("lost_reasons", 2),
    ("opportunities", 2),
    ("stage_history", 2),
    ("activities", 4),
    ("tasks", 5),
    ("saved_views", 7),
    ("tags", 8),
    ("record_tags", 8),
    ("custom_field_defs", 8),
    ("custom_field_options", 8),
    ("custom_field_values", 8),
    ("attachments", 10),
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
    ("attachments", "parent_type", "parent_id"),
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
    store: &AttachmentStore,
    path: &str,
    overwrite: bool,
) -> Result<ArchiveExportReport, ApplicationError> {
    let path = check_export_destination(storage, path, overwrite)?;
    let connection = storage.connection();

    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut record_counts = BTreeMap::new();
    for table in ARCHIVE_TABLES {
        let (rows, count) = export_table(connection, table)?;
        record_counts.insert((*table).to_owned(), count);
        files.push((format!("data/{table}.json"), rows));
    }
    // Managed attachment files travel with their rows; a row whose file is
    // gone would produce an archive that can never be imported, so it is an
    // error here rather than a surprise on the other side.
    for (id, file_name, relative_path) in attachment_files(connection)? {
        let managed = store.file_path(&relative_path);
        let bytes =
            std::fs::read(&managed).map_err(|error| ApplicationError::ValidationFailed {
                code: "attachment_file_missing",
                field: "path".into(),
                message: format!(
                    "attachment {id} (\"{file_name}\") is missing its file at {}: {error}",
                    managed.display()
                ),
            })?;
        files.push((format!("{ASSET_PREFIX}{id}/{file_name}"), bytes));
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
    // Always present, even with no attachments, so the layout is stable.
    writer
        .add_directory("assets", options)
        .map_err(zip_write_error)?;
    let archive_bytes = writer.finish().map_err(zip_write_error)?.into_inner();

    write_export_file(&path, &archive_bytes)?;

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

/// Every attachment's id, display name, and managed relative path.
fn attachment_files(
    connection: &Connection,
) -> Result<Vec<(String, String, String)>, ApplicationError> {
    let mut statement = connection
        .prepare("SELECT id, file_name, relative_path FROM attachments ORDER BY created_at, id")?;
    let rows = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
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

/// Caps on what an untrusted archive may expand to. Deflate hides its ratio
/// until decompression, so entries are read through a limit instead of trusted:
/// a real contractor database is a few megabytes, and these leave headroom.
pub const MAX_ENTRY_BYTES: u64 = 256 * 1024 * 1024;
const MAX_ARCHIVE_BYTES: u64 = 1024 * 1024 * 1024;

/// Most issues a preview reports; the payload crosses an IPC boundary, so a
/// pathological archive cannot turn into a pathological response.
const MAX_ISSUES: usize = 100;

/// One validated row: values in canonical column order.
struct ArchiveRow {
    values: Vec<Value>,
}

/// A fully verified archive: the manifest, the rows it carries, the counts
/// actually parsed, and every problem found. Rows are only trustworthy — and
/// import only allowed — when `issues` is empty.
struct VerifiedArchive {
    manifest: ArchiveManifest,
    tables: BTreeMap<&'static str, Vec<ArchiveRow>>,
    record_counts: BTreeMap<String, i64>,
    /// Verified attachment bytes keyed by managed relative path
    /// (`<attachment id>/<file name>`); empty unless the archive is clean.
    assets: BTreeMap<String, Vec<u8>>,
    issues: Vec<ArchiveIssue>,
}

/// Bounded issue collector: keeps the first `MAX_ISSUES` problems, counts the
/// rest, and tells verification when to stop looking.
struct IssueLog {
    issues: Vec<ArchiveIssue>,
    total: usize,
}

impl IssueLog {
    fn new() -> Self {
        Self {
            issues: Vec::new(),
            total: 0,
        }
    }

    fn push(&mut self, code: &str, message: String) {
        self.total += 1;
        if self.issues.len() < MAX_ISSUES {
            self.issues.push(ArchiveIssue {
                code: code.to_owned(),
                message,
            });
        }
    }

    fn is_empty(&self) -> bool {
        self.total == 0
    }

    /// True once the cap is reached; verification stops at the next boundary.
    fn is_full(&self) -> bool {
        self.issues.len() >= MAX_ISSUES
    }

    fn finish(mut self) -> Vec<ArchiveIssue> {
        if self.total > self.issues.len() {
            let hidden = self.total - self.issues.len();
            self.issues.push(ArchiveIssue {
                code: "too_many_issues".to_owned(),
                message: format!(
                    "stopped after {MAX_ISSUES} problems; {hidden} more were not reported"
                ),
            });
        }
        self.issues
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

/// Verify an archive end to end without writing anything: entry paths, size
/// caps, checksums, versions, row shapes, record counts, references, structural
/// minimums, and finally a full dry-run apply into a throwaway database.
fn verify_archive(
    connection: &Connection,
    path: &str,
) -> Result<VerifiedArchive, ApplicationError> {
    let file = std::fs::File::open(path).map_err(|error| unreadable(path, error.to_string()))?;
    let mut zip =
        zip::ZipArchive::new(file).map_err(|error| unreadable(path, error.to_string()))?;

    let mut issues = IssueLog::new();
    let entries = read_entries(&mut zip, path, &mut issues)?;
    if let Some(declared) = declared_entry_count(path)? {
        // Duplicate names collapse when the central directory is indexed, so
        // the declared count is what catches a smuggled second copy.
        if declared as usize != zip.len() {
            issues.push(
                "duplicate_entry",
                format!(
                    "archive declares {declared} entries but only {} are unique",
                    zip.len()
                ),
            );
        }
    }

    let manifest_bytes = entries
        .get("manifest.json")
        .ok_or_else(|| unreadable(path, "no manifest.json".into()))?;
    let manifest: ArchiveManifest = serde_json::from_slice(manifest_bytes)
        .map_err(|error| unreadable(path, format!("manifest.json is invalid: {error}")))?;

    let mut verified = VerifiedArchive {
        manifest,
        tables: BTreeMap::new(),
        record_counts: BTreeMap::new(),
        assets: BTreeMap::new(),
        issues: Vec::new(),
    };

    // A foreign product's archive is one clear problem, not sixteen missing
    // table files, so it short-circuits the rest of verification.
    if !verified
        .manifest
        .product
        .name
        .eq_ignore_ascii_case(env!("CARGO_PKG_NAME"))
    {
        issues.push(
            "wrong_product",
            format!(
                "archive was written by \"{}\", not ContractorCRM",
                verified.manifest.product.name
            ),
        );
        verified.issues = issues.finish();
        return Ok(verified);
    }

    verify_manifest_versions(&verified.manifest, &mut issues);
    verify_checksums(&verified.manifest, &entries, &mut issues);

    if !issues.is_full() {
        for table in ARCHIVE_TABLES {
            let columns = table_columns(connection, table)?;
            let file = format!("data/{table}.json");
            let Some(content) = entries.get(&file) else {
                if !table_predates_archive(table, verified.manifest.database_migration_version) {
                    issues.push("missing_table_file", format!("archive has no {file}"));
                }
                continue;
            };
            if let Some(rows) = parse_table(table, &columns, content, &mut issues) {
                verified
                    .record_counts
                    .insert((*table).to_owned(), rows.len() as i64);
                verified.tables.insert(*table, rows);
            }
            if issues.is_full() {
                break;
            }
        }
        verify_record_counts(&verified.manifest, &verified.record_counts, &mut issues);
    }

    // References, assets, structure, and the dry run only mean anything once
    // the rows themselves parsed cleanly.
    if issues.is_empty() {
        verify_references(connection, &verified.tables, &mut issues)?;
    }
    if issues.is_empty() {
        verify_structure(connection, &verified.tables, &mut issues)?;
    }
    if issues.is_empty() {
        verified.assets = verify_assets(connection, &verified.tables, &entries, &mut issues)?;
    }
    if issues.is_empty() {
        dry_run_apply(connection, &verified.tables, &mut issues)?;
    }

    verified.issues = issues.finish();
    Ok(verified)
}

/// Read every acceptable entry into memory under the size caps. Files under
/// `assets/` are attachment bytes: they are read like any other entry, so the
/// manifest checksum and the owning attachments row both get to verify them.
fn read_entries<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
    path: &str,
    issues: &mut IssueLog,
) -> Result<BTreeMap<String, Vec<u8>>, ApplicationError> {
    let mut entries: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut total_bytes: u64 = 0;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|error| unreadable(path, error.to_string()))?;
        let name = entry.name().to_owned();
        if let Some(problem) = entry_path_issue(&name) {
            issues.push(&problem.code, problem.message);
            continue;
        }
        if entry.is_dir() {
            continue;
        }
        // Read through a limit rather than trusting the declared size: a
        // compression bomb only lies once it is decompressed. Aborted reads
        // still spend their budget, so total work stays under the archive cap
        // no matter how many oversized entries an attacker packs.
        let remaining = MAX_ARCHIVE_BYTES.saturating_sub(total_bytes);
        if remaining == 0 {
            issues.push(
                "archive_too_large",
                format!("archive expands past the {MAX_ARCHIVE_BYTES} byte limit"),
            );
            break;
        }
        let cap = MAX_ENTRY_BYTES.min(remaining);
        let mut content = Vec::new();
        let read = entry
            .by_ref()
            .take(cap + 1)
            .read_to_end(&mut content)
            .map_err(|error| unreadable(path, error.to_string()))? as u64;
        total_bytes += read;
        if read > cap {
            // Drop the partial read; the entry is refused, not truncated.
            drop(content);
            issues.push(
                "entry_too_large",
                format!("entry \"{name}\" is larger than the {MAX_ENTRY_BYTES} byte limit"),
            );
            continue;
        }
        entries.insert(name, content);
        if issues.is_full() {
            break;
        }
    }
    Ok(entries)
}

/// Reject anything that could escape the archive root; `csv/` and `assets/`
/// are carried but never applied, everything else must be a known data file.
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

fn issue(code: &str, message: String) -> ArchiveIssue {
    ArchiveIssue {
        code: code.to_owned(),
        message,
    }
}

/// Number of central directory records the archive claims, read from the tail
/// of the file. `None` when it cannot be read (ZIP64).
fn declared_entry_count(path: &str) -> Result<Option<u16>, ApplicationError> {
    use std::io::Seek;

    let mut file =
        std::fs::File::open(path).map_err(|error| unreadable(path, error.to_string()))?;
    let length = file
        .metadata()
        .map_err(|error| unreadable(path, error.to_string()))?
        .len();
    // The end of central directory record is at most 22 bytes plus a 64 KiB
    // comment, so only the tail has to be read.
    let tail_length = length.min(u16::MAX as u64 + 22);
    file.seek(std::io::SeekFrom::Start(length - tail_length))
        .map_err(|error| unreadable(path, error.to_string()))?;
    let mut tail = Vec::with_capacity(tail_length as usize);
    file.take(tail_length)
        .read_to_end(&mut tail)
        .map_err(|error| unreadable(path, error.to_string()))?;

    let signature = [0x50, 0x4b, 0x05, 0x06];
    let Some(offset) = tail.windows(4).rposition(|window| window == signature) else {
        return Ok(None);
    };
    let (Some(low), Some(high)) = (tail.get(offset + 10), tail.get(offset + 11)) else {
        return Ok(None);
    };
    let count = u16::from_le_bytes([*low, *high]);
    Ok((count != u16::MAX).then_some(count))
}

fn verify_manifest_versions(manifest: &ArchiveManifest, issues: &mut IssueLog) {
    if manifest.schema_version != ARCHIVE_SCHEMA_VERSION {
        issues.push(
            "unsupported_schema_version",
            format!(
                "archive schema version {} is not supported (expected {ARCHIVE_SCHEMA_VERSION})",
                manifest.schema_version
            ),
        );
    }
    let supported = storage::latest_migration_version();
    if manifest.database_migration_version > supported {
        issues.push(
            "unsupported_migration_version",
            format!(
                "archive was written at database version {} but this app supports {supported}; \
                 update the app first",
                manifest.database_migration_version
            ),
        );
    }
}

/// Every listed file must be present and match its checksum, and every file
/// present must be listed — nothing enters the import unchecksummed.
fn verify_checksums(
    manifest: &ArchiveManifest,
    entries: &BTreeMap<String, Vec<u8>>,
    issues: &mut IssueLog,
) {
    let mut listed = BTreeSet::new();
    for file in &manifest.files {
        listed.insert(file.path.clone());
        let Some(content) = entries.get(&file.path) else {
            issues.push(
                "missing_file",
                format!(
                    "manifest lists \"{}\" but the archive has no such file",
                    file.path
                ),
            );
            continue;
        };
        if content.len() as u64 != file.bytes {
            issues.push(
                "size_mismatch",
                format!(
                    "\"{}\" is {} bytes but the manifest says {}",
                    file.path,
                    content.len(),
                    file.bytes
                ),
            );
        }
        if sha256_hex(content) != file.sha256 {
            issues.push(
                "checksum_mismatch",
                format!("\"{}\" does not match its manifest checksum", file.path),
            );
        }
        if issues.is_full() {
            return;
        }
    }
    for name in entries.keys() {
        if name == "manifest.json" || listed.contains(name) {
            continue;
        }
        issues.push(
            "unlisted_file",
            format!("\"{name}\" is in the archive but not listed in the manifest"),
        );
        if issues.is_full() {
            return;
        }
    }
}

/// The manifest's claimed counts must match the rows actually parsed — an
/// inflated count over an emptied data file would otherwise import as a wipe.
fn verify_record_counts(
    manifest: &ArchiveManifest,
    parsed: &BTreeMap<String, i64>,
    issues: &mut IssueLog,
) {
    for table in ARCHIVE_TABLES {
        let Some(parsed_count) = parsed.get(*table) else {
            continue; // the table file is already reported as missing or unusable
        };
        match manifest.record_counts.get(*table) {
            Some(claimed) if claimed == parsed_count => {}
            Some(claimed) => issues.push(
                "record_count_mismatch",
                format!(
                    "manifest claims {claimed} {table} rows but the archive holds {parsed_count}"
                ),
            ),
            None => issues.push(
                "record_count_mismatch",
                format!("manifest has no record count for {table}"),
            ),
        }
    }
    for table in manifest.record_counts.keys() {
        if !ARCHIVE_TABLES.contains(&table.as_str()) {
            issues.push(
                "record_count_mismatch",
                format!("manifest counts unknown table \"{table}\""),
            );
        }
    }
}

/// Parse and shape-check one table file. Returns None when the file itself is
/// unusable, so callers stop looking at that table.
fn parse_table(
    table: &str,
    columns: &[ColumnSpec],
    content: &[u8],
    issues: &mut IssueLog,
) -> Option<Vec<ArchiveRow>> {
    let parsed: serde_json::Value = match serde_json::from_slice(content) {
        Ok(value) => value,
        Err(error) => {
            issues.push(
                "invalid_table_json",
                format!("data/{table}.json is not valid JSON: {error}"),
            );
            return None;
        }
    };
    let Some(array) = parsed.as_array() else {
        issues.push(
            "invalid_table_json",
            format!("data/{table}.json is not a JSON array"),
        );
        return None;
    };

    let mut rows = Vec::with_capacity(array.len());
    let mut seen_keys = BTreeSet::new();
    for (index, item) in array.iter().enumerate() {
        if issues.is_full() {
            break;
        }
        let Some(object) = item.as_object() else {
            issues.push(
                "invalid_table_json",
                format!("{table} row {index} is not a JSON object"),
            );
            continue;
        };
        for key in object.keys() {
            if !columns.iter().any(|column| &column.camel == key) {
                issues.push(
                    "unknown_column",
                    format!("{table} row {index} has unknown field \"{key}\""),
                );
            }
        }
        let mut values = Vec::with_capacity(columns.len());
        let mut row_valid = true;
        for column in columns {
            match archive_value(table, index, column, object.get(&column.camel)) {
                Ok(value) => values.push(value),
                Err(problem) => {
                    issues.push(&problem.code, problem.message);
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
                        issues.push(
                            "invalid_id",
                            format!("{table} row {index} has an empty {}", column.camel),
                        );
                        row_valid = false;
                    }
                }
            }
            if column.name == "version" {
                if let Value::Integer(number) = &values[position] {
                    if *number < 1 {
                        issues.push(
                            "invalid_version",
                            format!("{table} row {index} has version {number}"),
                        );
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
            issues.push(
                "duplicate_primary_key",
                format!("{table} has more than one row with key \"{key}\""),
            );
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

/// Column specs for every canonical table, read once per verification.
fn all_table_columns(
    connection: &Connection,
) -> Result<BTreeMap<&'static str, Vec<ColumnSpec>>, ApplicationError> {
    let mut specs = BTreeMap::new();
    for table in ARCHIVE_TABLES {
        specs.insert(*table, table_columns(connection, table)?);
    }
    Ok(specs)
}

/// Text value of one column in a parsed row, if it is set.
fn cell(
    specs: &BTreeMap<&'static str, Vec<ColumnSpec>>,
    table: &str,
    row: &ArchiveRow,
    column: &str,
) -> Option<String> {
    let position = specs
        .get(table)?
        .iter()
        .position(|spec| spec.name == column)?;
    match &row.values[position] {
        Value::Text(text) => Some(text.clone()),
        _ => None,
    }
}

/// Validate every relationship among the archived tables in memory, so a
/// dangling reference is reported instead of failing mid-write.
fn verify_references(
    connection: &Connection,
    tables: &BTreeMap<&'static str, Vec<ArchiveRow>>,
    issues: &mut IssueLog,
) -> Result<(), ApplicationError> {
    let specs = all_table_columns(connection)?;
    let mut ids: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    for table in ARCHIVE_TABLES {
        if let (Some(rows), Some(position)) = (
            tables.get(table),
            specs[table].iter().position(|column| column.name == "id"),
        ) {
            let mut set = BTreeSet::new();
            for row in rows {
                if let Value::Text(text) = &row.values[position] {
                    set.insert(text.clone());
                }
            }
            ids.insert(*table, set);
        }
    }

    for (table, column, target) in ARCHIVE_FOREIGN_KEYS {
        let Some(rows) = tables.get(table) else {
            continue;
        };
        for (index, row) in rows.iter().enumerate() {
            let Some(value) = cell(&specs, table, row, column) else {
                continue; // NULL references are allowed where the column is nullable
            };
            if !ids.get(target).is_some_and(|set| set.contains(&value)) {
                issues.push(
                    "missing_reference",
                    format!(
                        "{table} row {index} references {target} \"{value}\", which the archive \
                         does not contain"
                    ),
                );
                if issues.is_full() {
                    return Ok(());
                }
            }
        }
    }

    for (table, type_column, id_column) in ARCHIVE_POLYMORPHIC_KEYS {
        let Some(rows) = tables.get(table) else {
            continue;
        };
        for (index, row) in rows.iter().enumerate() {
            let (Some(parent_type), Some(parent_id)) = (
                cell(&specs, table, row, type_column),
                cell(&specs, table, row, id_column),
            ) else {
                continue; // both null together, enforced by the table CHECK
            };
            let Some(target) = polymorphic_table(&parent_type) else {
                issues.push(
                    "unknown_parent_type",
                    format!("{table} row {index} has unknown {type_column} \"{parent_type}\""),
                );
                continue;
            };
            if !ids.get(target).is_some_and(|set| set.contains(&parent_id)) {
                issues.push(
                    "missing_reference",
                    format!(
                        "{table} row {index} references {target} \"{parent_id}\", which the \
                         archive does not contain"
                    ),
                );
            }
            if issues.is_full() {
                return Ok(());
            }
        }
    }
    Ok(())
}

/// True when the archive was written before the table existed, which is the
/// only case where a missing data file is normal rather than a problem.
fn table_predates_archive(table: &str, archive_migration_version: i64) -> bool {
    TABLE_INTRODUCED_IN
        .iter()
        .find(|(name, _)| *name == table)
        .is_some_and(|(_, introduced)| archive_migration_version < *introduced)
}

/// Attachment files and attachments rows must match one for one: every row
/// needs its bytes, every `assets/` entry needs its row, and the bytes have to
/// match the size and checksum the row recorded (on top of the manifest
/// checksum every archived file already passes). Returns the verified bytes
/// keyed by managed relative path.
fn verify_assets(
    connection: &Connection,
    tables: &BTreeMap<&'static str, Vec<ArchiveRow>>,
    entries: &BTreeMap<String, Vec<u8>>,
    issues: &mut IssueLog,
) -> Result<BTreeMap<String, Vec<u8>>, ApplicationError> {
    let specs = all_table_columns(connection)?;
    let empty = Vec::new();
    let rows = tables.get("attachments").unwrap_or(&empty);

    let mut verified = BTreeMap::new();
    let mut expected = BTreeSet::new();
    for (index, row) in rows.iter().enumerate() {
        let (Some(id), Some(file_name)) = (
            cell(&specs, "attachments", row, "id"),
            cell(&specs, "attachments", row, "file_name"),
        ) else {
            continue; // both columns are NOT NULL, so parsing already reported this
        };
        let relative_path = format!("{id}/{file_name}");
        let entry_name = format!("{ASSET_PREFIX}{relative_path}");
        expected.insert(entry_name.clone());
        let Some(content) = entries.get(&entry_name) else {
            issues.push(
                "attachment_file_missing",
                format!("attachments row {index} has no \"{entry_name}\" in the archive"),
            );
            continue;
        };
        let claimed_size = number_cell(&specs, "attachments", row, "size_bytes");
        if claimed_size != Some(content.len() as i64) {
            issues.push(
                "attachment_size_mismatch",
                format!(
                    "\"{entry_name}\" is {} bytes but its row records {}",
                    content.len(),
                    claimed_size.unwrap_or_default()
                ),
            );
            continue;
        }
        if cell(&specs, "attachments", row, "sha256").as_deref() != Some(&sha256_hex(content)) {
            issues.push(
                "attachment_checksum_mismatch",
                format!("\"{entry_name}\" does not match the checksum its row records"),
            );
            continue;
        }
        verified.insert(relative_path, content.clone());
        if issues.is_full() {
            return Ok(BTreeMap::new());
        }
    }

    for name in entries.keys() {
        if !name.starts_with(ASSET_PREFIX) || expected.contains(name) {
            continue;
        }
        issues.push(
            "unexpected_asset",
            format!("\"{name}\" has no attachments row that claims it"),
        );
        if issues.is_full() {
            break;
        }
    }
    if !issues.is_empty() {
        // Only a clean archive hands its bytes on to the import.
        return Ok(BTreeMap::new());
    }
    Ok(verified)
}

/// Integer value of one column in a parsed row, if it is set.
fn number_cell(
    specs: &BTreeMap<&'static str, Vec<ColumnSpec>>,
    table: &str,
    row: &ArchiveRow,
    column: &str,
) -> Option<i64> {
    let position = specs
        .get(table)?
        .iter()
        .position(|spec| spec.name == column)?;
    match &row.values[position] {
        Value::Integer(number) => Some(*number),
        _ => None,
    }
}

/// The pipeline shape the app itself seeds and depends on: without a pipeline
/// whose stages cover open, won, and lost, creating an opportunity is
/// impossible and the imported database would be unusable.
fn verify_structure(
    connection: &Connection,
    tables: &BTreeMap<&'static str, Vec<ArchiveRow>>,
    issues: &mut IssueLog,
) -> Result<(), ApplicationError> {
    let specs = all_table_columns(connection)?;
    let empty = Vec::new();
    let pipelines = tables.get("pipelines").unwrap_or(&empty);
    let stages = tables.get("stages").unwrap_or(&empty);
    if pipelines.is_empty() {
        issues.push(
            "missing_pipeline",
            "the archive has no pipelines; at least one is required".to_owned(),
        );
        return Ok(());
    }

    let mut complete_pipelines = 0;
    for (index, pipeline) in pipelines.iter().enumerate() {
        let Some(pipeline_id) = cell(&specs, "pipelines", pipeline, "id") else {
            continue;
        };
        let kinds = stages
            .iter()
            .filter(|stage| {
                cell(&specs, "stages", stage, "pipeline_id").as_deref() == Some(&pipeline_id)
            })
            .filter_map(|stage| cell(&specs, "stages", stage, "kind"))
            .collect::<BTreeSet<_>>();
        if kinds.is_empty() {
            issues.push(
                "missing_pipeline",
                format!("pipeline {index} (\"{pipeline_id}\") has no stages"),
            );
            continue;
        }
        if ["open", "won", "lost"]
            .iter()
            .all(|kind| kinds.contains(*kind))
        {
            complete_pipelines += 1;
        }
    }
    if complete_pipelines == 0 {
        issues.push(
            "missing_stage_kind",
            "no pipeline has open, won, and lost stages".to_owned(),
        );
    }
    Ok(())
}

/// Apply the whole replace into a throwaway in-memory database. UNIQUE and
/// CHECK constraints, triggers, and the search rebuild are all exercised here,
/// so "no issues" really does mean the real import will succeed.
fn dry_run_apply(
    connection: &Connection,
    tables: &BTreeMap<&'static str, Vec<ArchiveRow>>,
    issues: &mut IssueLog,
) -> Result<(), ApplicationError> {
    let specs = all_table_columns(connection)?;
    let mut scratch = Storage::open_in_memory()?;
    let transaction = immediate(&mut scratch)?;
    if let Err((table, error)) = replace_rows(&transaction, &specs, tables) {
        issues.push(
            "constraint_violation",
            format!("{table} cannot be imported: {error}"),
        );
    } else if let Err(error) = rebuild_search_index(&transaction) {
        issues.push(
            "constraint_violation",
            format!("the search index cannot be rebuilt: {error}"),
        );
    }
    // Never committed: the scratch database exists only to be thrown away.
    drop(transaction);
    Ok(())
}

/// Delete every canonical row and insert the archive's, in dependency order.
/// The error carries the table so a failure can be reported in the caller's
/// terms rather than as a bare SQLite message.
fn replace_rows(
    transaction: &rusqlite::Transaction<'_>,
    specs: &BTreeMap<&'static str, Vec<ColumnSpec>>,
    tables: &BTreeMap<&'static str, Vec<ArchiveRow>>,
) -> Result<BTreeMap<String, i64>, (&'static str, ApplicationError)> {
    for table in ARCHIVE_TABLES.iter().rev() {
        transaction
            .execute(&format!("DELETE FROM \"{table}\""), [])
            .map_err(|error| (*table, ApplicationError::from(error)))?;
    }
    let mut record_counts = BTreeMap::new();
    for table in ARCHIVE_TABLES {
        let columns = &specs[table];
        let rows = tables.get(table).map(Vec::as_slice).unwrap_or(&[]);
        let names = columns
            .iter()
            .map(|column| format!("\"{}\"", column.name))
            .collect::<Vec<_>>()
            .join(", ");
        let placeholders = (1..=columns.len())
            .map(|position| format!("?{position}"))
            .collect::<Vec<_>>()
            .join(", ");
        let mut statement = transaction
            .prepare(&format!(
                "INSERT INTO \"{table}\" ({names}) VALUES ({placeholders})"
            ))
            .map_err(|error| (*table, ApplicationError::from(error)))?;
        for row in rows {
            statement
                .execute(rusqlite::params_from_iter(row.values.iter()))
                .map_err(|error| (*table, ApplicationError::from(error)))?;
        }
        drop(statement);
        record_counts.insert((*table).to_owned(), rows.len() as i64);
    }
    Ok(record_counts)
}

// ---------------------------------------------------------------------------
// Preview and import
// ---------------------------------------------------------------------------

/// Verify an archive and report what it actually holds — the counts come from
/// the parsed rows, never from the manifest's claim. Never writes.
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
        record_counts: verified.record_counts,
        issues: verified.issues,
    })
}

/// Replace every canonical record with the archive's. Verification (including
/// the dry run) happens first, then a timestamped safety backup, then one
/// transaction. A refused or failed import leaves the live database — and the
/// filesystem — exactly as it was.
pub fn import_archive(
    storage: &mut Storage,
    store: &AttachmentStore,
    path: &str,
) -> Result<ArchiveImportReport, ApplicationError> {
    let verified = verify_archive(storage.connection(), path)?;
    if !verified.issues.is_empty() {
        return Err(archive_invalid(&verified.issues));
    }
    let specs = all_table_columns(storage.connection())?;

    // Attachment bytes land in a staging directory first: everything that can
    // fail on the filesystem fails before the database transaction, and the
    // swap afterwards is only renames.
    let staging = stage_assets(store, &verified.assets)?;

    // Only now, with nothing left to reject, is the live database touched.
    let safety_backup_path = match storage.safety_copy("pre-import") {
        Ok(path) => path,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error.into());
        }
    };
    let transaction = immediate(storage)?;
    let outcome = replace_rows(&transaction, &specs, &verified.tables)
        .map_err(|(table, error)| {
            issue(
                "constraint_violation",
                format!("{table} cannot be imported: {error}"),
            )
        })
        .and_then(|record_counts| {
            rebuild_search_index(&transaction)
                .map(|()| record_counts)
                .map_err(|error| {
                    issue(
                        "constraint_violation",
                        format!("the search index cannot be rebuilt: {error}"),
                    )
                })
        });
    let record_counts = match outcome {
        Ok(record_counts) => record_counts,
        Err(problem) => {
            // The dry run passed, so this is unexpected — roll back and take the
            // untouched safety copy and staging directory with us rather than
            // leaving orphans.
            drop(transaction);
            let _ = std::fs::remove_file(&safety_backup_path);
            let _ = std::fs::remove_dir_all(&staging);
            return Err(archive_invalid(&[problem]));
        }
    };

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

    // Ordering is deliberate: the database is authoritative, so the managed
    // files are swapped only after the rows they belong to are committed. A
    // failure here is reported, but the import itself already happened.
    swap_staged_assets(store, &staging)?;

    Ok(ArchiveImportReport {
        record_counts,
        safety_backup_path: safety_backup_path.to_string_lossy().into_owned(),
    })
}

/// Write verified attachment bytes into a fresh staging directory under the
/// attachments root. Any failure clears the staging directory so a refused
/// import leaves the filesystem exactly as it was.
fn stage_assets(
    store: &AttachmentStore,
    assets: &BTreeMap<String, Vec<u8>>,
) -> Result<std::path::PathBuf, ApplicationError> {
    store.ensure_root()?;
    let staging = store
        .absolute_root()
        .join(format!("{IMPORT_STAGING_PREFIX}{}", storage::new_id()));
    let staged = (|| -> Result<(), ApplicationError> {
        std::fs::create_dir_all(&staging)?;
        for (relative_path, bytes) in assets {
            let destination = relative_path
                .split('/')
                .fold(staging.clone(), |path, component| path.join(component));
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&destination, bytes)?;
        }
        Ok(())
    })();
    if let Err(error) = staged {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(error);
    }
    Ok(staging)
}

/// Replace the managed attachment files with the staged ones: clear the old
/// directories, move the staged ones into place, then drop the staging
/// directory. Called only after the import transaction commits.
fn swap_staged_assets(
    store: &AttachmentStore,
    staging: &std::path::Path,
) -> Result<(), ApplicationError> {
    let root = store.absolute_root();
    for entry in std::fs::read_dir(&root)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(IMPORT_STAGING_PREFIX) {
            continue; // this import's staging, or a leftover from a crashed one
        }
        if entry.file_type()?.is_dir() {
            std::fs::remove_dir_all(entry.path())?;
        } else {
            std::fs::remove_file(entry.path())?;
        }
    }
    for entry in std::fs::read_dir(staging)? {
        let entry = entry?;
        std::fs::rename(entry.path(), root.join(entry.file_name()))?;
    }
    std::fs::remove_dir_all(staging)?;
    Ok(())
}

/// Turn verification issues into the single refusal the caller sees.
fn archive_invalid(issues: &[ArchiveIssue]) -> ApplicationError {
    let first = issues
        .first()
        .map(|problem| problem.message.clone())
        .unwrap_or_else(|| "the archive cannot be imported".to_owned());
    let suffix = match issues.len().saturating_sub(1) {
        0 => String::new(),
        1 => " (and 1 more problem)".to_owned(),
        more => format!(" (and {more} more problems)"),
    };
    ApplicationError::ValidationFailed {
        code: "archive_invalid",
        field: "path".into(),
        message: format!("{first}{suffix}"),
    }
}
