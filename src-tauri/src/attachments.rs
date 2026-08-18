//! Attachments as managed files (docs/FEATURES.md "Attachments").
//!
//! A contact or opportunity attachment is a copy of the user's file inside the
//! application's own attachments root, laid out as
//! `<root>/<attachment id>/<file name>`. The database row is the record of
//! truth: it carries the display name, media type, size, and SHA-256 of the
//! managed copy, and the file name it stores is already sanitized so every
//! consumer — the archive exporter included — can build a safe path from it.

use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::application::{check_version, immediate, log_command};
use crate::archive::MAX_ENTRY_BYTES;
use crate::domain::Actor;
use crate::error::ApplicationError;
use crate::storage::{new_id, now_utc, Storage};

/// Directory holding managed attachment files inside the app data directory.
pub const ATTACHMENTS_DIR_NAME: &str = "attachments";

/// Largest file the CRM will take under management. Shared with the archive
/// per-entry cap so anything that can be attached can also be exported.
pub const MAX_ATTACHMENT_BYTES: u64 = MAX_ENTRY_BYTES;

/// Longest managed file name in bytes; long names break on some filesystems
/// and in archive tooling, so they are truncated (extension preserved).
const MAX_FILE_NAME_BYTES: usize = 120;

/// Device names Windows still refuses to use as file names, whatever the
/// extension. Guarded on every platform so archives stay portable.
const WINDOWS_RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Prefix of the staging directory an archive import writes into before it
/// swaps managed files. Skipped when old attachment directories are cleared.
pub(crate) const IMPORT_STAGING_PREFIX: &str = ".import-staging-";

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

/// Records that can own attachments in v1.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AttachmentParentType {
    Contact,
    Opportunity,
}

impl AttachmentParentType {
    pub(crate) fn as_database_value(self) -> &'static str {
        match self {
            Self::Contact => "contact",
            Self::Opportunity => "opportunity",
        }
    }

    fn from_database_value(value: &str) -> Option<Self> {
        match value {
            "contact" => Some(Self::Contact),
            "opportunity" => Some(Self::Opportunity),
            _ => None,
        }
    }
}

/// One managed file. `relative_path` stays internal — callers open attachments
/// through `attachment_path`, never by building a path themselves.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Attachment {
    pub id: String,
    pub parent_type: AttachmentParentType,
    pub parent_id: String,
    pub file_name: String,
    pub media_type: Option<String>,
    pub size_bytes: i64,
    pub sha256: String,
    pub created_at: String,
    pub version: i64,
}

/// Take a copy of an existing file on disk under management.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AddAttachmentRequest {
    #[serde(default)]
    pub actor: Actor,
    pub parent_type: AttachmentParentType,
    pub parent_id: String,
    pub source_path: String,
}

/// Drop an attachment row and, best effort, its managed file.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoveAttachmentRequest {
    #[serde(default)]
    pub actor: Actor,
    pub attachment_id: String,
    pub expected_version: i64,
}

/// Whether the managed file went with the row. A left-behind file is harmless
/// — the row is gone, so nothing lists or exports it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttachmentRemoval {
    pub file_removed: bool,
}

/// Absolute location of a managed file, for the frontend to open with the OS.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttachmentLocation {
    pub path: String,
    pub exists: bool,
}

// ---------------------------------------------------------------------------
// Managed store
// ---------------------------------------------------------------------------

/// Owns the attachments root directory. Constructed with an explicit path (the
/// test seam) or from the app data directory in production, mirroring how
/// `Storage` is opened.
pub struct AttachmentStore {
    root: PathBuf,
}

impl AttachmentStore {
    /// Root the store at an explicit directory; it is created on first write.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Production layout: `<app data>/attachments`.
    pub fn open_in_app_data(app_data_dir: impl AsRef<Path>) -> Self {
        Self::new(app_data_dir.as_ref().join(ATTACHMENTS_DIR_NAME))
    }

    /// Where managed files live.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Absolute form of the root, so paths handed to the OS always resolve.
    pub fn absolute_root(&self) -> PathBuf {
        std::path::absolute(&self.root).unwrap_or_else(|_| self.root.clone())
    }

    /// Create the root if it does not exist yet.
    pub(crate) fn ensure_root(&self) -> Result<(), ApplicationError> {
        std::fs::create_dir_all(&self.root)?;
        Ok(())
    }

    /// Absolute path of one managed file from its stored relative path.
    pub(crate) fn file_path(&self, relative_path: &str) -> PathBuf {
        let mut path = self.absolute_root();
        for component in relative_path.split('/') {
            path.push(component);
        }
        path
    }

    /// Directory holding one attachment's file.
    pub(crate) fn directory(&self, attachment_id: &str) -> PathBuf {
        self.absolute_root().join(attachment_id)
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Copy `sourcePath` under management and record it against its parent. The
/// file is copied (and hashed) before the row is written; a failed write takes
/// the copy with it, so a managed file never outlives a failed command.
pub fn add_attachment(
    storage: &mut Storage,
    store: &AttachmentStore,
    request: AddAttachmentRequest,
) -> Result<Attachment, ApplicationError> {
    let parent_id = request.parent_id.trim().to_owned();
    if parent_id.is_empty() {
        return Err(invalid_input("parentId", "parentId is required".into()));
    }
    require_active_parent(storage.connection(), request.parent_type, &parent_id)?;

    let source = PathBuf::from(&request.source_path);
    let metadata = std::fs::metadata(&source).map_err(|error| {
        invalid_input(
            "sourcePath",
            format!("cannot read \"{}\": {error}", request.source_path),
        )
    })?;
    if !metadata.is_file() {
        return Err(invalid_input(
            "sourcePath",
            format!("\"{}\" is not a file", request.source_path),
        ));
    }
    if metadata.len() > MAX_ATTACHMENT_BYTES {
        return Err(too_large(metadata.len()));
    }

    store.ensure_root()?;
    if is_inside_managed_root(store, &source) {
        return Err(invalid_input(
            "sourcePath",
            format!(
                "\"{}\" is already a managed attachment; attach the original file instead",
                request.source_path
            ),
        ));
    }

    let raw_name = source
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| invalid_input("sourcePath", "the file name is not valid UTF-8".into()))?;
    let file_name = sanitized_file_name(raw_name)?;

    let id = new_id();
    let relative_path = format!("{id}/{file_name}");
    let destination = store.file_path(&relative_path);
    std::fs::create_dir_all(store.directory(&id))?;
    let copied = copy_into_management(&source, &destination);
    let (size_bytes, sha256) = match copied {
        Ok(copied) => copied,
        Err(error) => {
            remove_managed_directory(store, &id);
            return Err(error);
        }
    };

    let attachment = Attachment {
        id,
        parent_type: request.parent_type,
        parent_id,
        media_type: Some(media_type_for(&file_name)),
        file_name,
        size_bytes: size_bytes as i64,
        sha256,
        created_at: now_utc(),
        version: 1,
    };

    // Row and audit entry in one transaction; a failure removes the copy so the
    // filesystem never keeps a file no row points at.
    let written = insert_attachment(storage, &attachment, &relative_path, request.actor);
    if let Err(error) = written {
        remove_managed_directory(store, &attachment.id);
        return Err(error);
    }
    Ok(attachment)
}

fn insert_attachment(
    storage: &mut Storage,
    attachment: &Attachment,
    relative_path: &str,
    actor: Actor,
) -> Result<(), ApplicationError> {
    let transaction = immediate(storage)?;
    transaction.execute(
        "INSERT INTO attachments (
            id, parent_type, parent_id, file_name, relative_path,
            media_type, size_bytes, sha256, created_at, version
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            attachment.id,
            attachment.parent_type.as_database_value(),
            attachment.parent_id,
            attachment.file_name,
            relative_path,
            attachment.media_type,
            attachment.size_bytes,
            attachment.sha256,
            attachment.created_at,
            attachment.version,
        ],
    )?;
    log_command(
        &transaction,
        actor,
        "attachment",
        &attachment.id,
        &format!(
            "attached \"{}\" to {} {}",
            attachment.file_name,
            attachment.parent_type.as_database_value(),
            attachment.parent_id
        ),
    )?;
    transaction.commit()?;
    Ok(())
}

/// Every attachment on one record, oldest first.
pub fn list_attachments(
    storage: &Storage,
    parent_type: AttachmentParentType,
    parent_id: &str,
) -> Result<Vec<Attachment>, ApplicationError> {
    let mut statement = storage.connection().prepare(
        "SELECT id, parent_type, parent_id, file_name, media_type, size_bytes,
                sha256, created_at, version
         FROM attachments
         WHERE parent_type = ?1 AND parent_id = ?2
         ORDER BY created_at, id",
    )?;
    let rows = statement
        .query_map(params![parent_type.as_database_value(), parent_id], |row| {
            attachment_from_row(row)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter().collect::<Result<Vec<_>, _>>()
}

/// Drop the row, then the file. The row is authoritative: once the transaction
/// commits the attachment is gone even if its bytes cannot be deleted.
pub fn remove_attachment(
    storage: &mut Storage,
    store: &AttachmentStore,
    request: RemoveAttachmentRequest,
) -> Result<AttachmentRemoval, ApplicationError> {
    let transaction = immediate(storage)?;
    let (attachment, _) = require_attachment(&transaction, &request.attachment_id)?;
    check_version(
        "attachment",
        &attachment.id,
        request.expected_version,
        attachment.version,
    )?;
    transaction.execute("DELETE FROM attachments WHERE id = ?1", [&attachment.id])?;
    log_command(
        &transaction,
        request.actor,
        "attachment",
        &attachment.id,
        &format!(
            "removed attachment \"{}\" from {} {}",
            attachment.file_name,
            attachment.parent_type.as_database_value(),
            attachment.parent_id
        ),
    )?;
    transaction.commit()?;

    Ok(AttachmentRemoval {
        file_removed: remove_managed_directory(store, &attachment.id),
    })
}

/// Absolute path of a managed file plus whether it is still on disk, so the
/// frontend can open it with the OS or explain why it cannot.
pub fn attachment_path(
    storage: &Storage,
    store: &AttachmentStore,
    attachment_id: &str,
) -> Result<AttachmentLocation, ApplicationError> {
    let relative_path: Option<String> = storage
        .connection()
        .query_row(
            "SELECT relative_path FROM attachments WHERE id = ?1",
            [attachment_id],
            |row| row.get(0),
        )
        .optional()?;
    let relative_path = relative_path.ok_or_else(|| ApplicationError::NotFound {
        resource: "attachment",
        id: attachment_id.into(),
    })?;
    let path = store.file_path(&relative_path);
    Ok(AttachmentLocation {
        exists: path.is_file(),
        path: path.to_string_lossy().into_owned(),
    })
}

// ---------------------------------------------------------------------------
// Shared helpers (also used by the archive seam)
// ---------------------------------------------------------------------------

/// Load one attachment row with its internal relative path.
pub(crate) fn require_attachment(
    connection: &Connection,
    attachment_id: &str,
) -> Result<(Attachment, String), ApplicationError> {
    let row = connection
        .query_row(
            "SELECT id, parent_type, parent_id, file_name, media_type, size_bytes,
                    sha256, created_at, version, relative_path
             FROM attachments WHERE id = ?1",
            [attachment_id],
            |row| Ok((attachment_from_row(row)?, row.get::<_, String>(9)?)),
        )
        .optional()?;
    let (attachment, relative_path) = row.ok_or_else(|| ApplicationError::NotFound {
        resource: "attachment",
        id: attachment_id.into(),
    })?;
    Ok((attachment?, relative_path))
}

/// Row shape shared by every attachment read; parsing the parent type is
/// deferred so a corrupt value surfaces as invalid stored data, not a panic.
fn attachment_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<Result<Attachment, ApplicationError>> {
    let parent_type_text: String = row.get(1)?;
    let Some(parent_type) = AttachmentParentType::from_database_value(&parent_type_text) else {
        return Ok(Err(ApplicationError::InvalidStoredData(format!(
            "attachment parent type \"{parent_type_text}\" is not supported"
        ))));
    };
    Ok(Ok(Attachment {
        id: row.get(0)?,
        parent_type,
        parent_id: row.get(2)?,
        file_name: row.get(3)?,
        media_type: row.get(4)?,
        size_bytes: row.get(5)?,
        sha256: row.get(6)?,
        created_at: row.get(7)?,
        version: row.get(8)?,
    }))
}

/// The parent has to exist and be active — attaching to an archived record
/// would hide the file behind a record the user has put away.
fn require_active_parent(
    connection: &Connection,
    parent_type: AttachmentParentType,
    parent_id: &str,
) -> Result<(), ApplicationError> {
    let sql = match parent_type {
        AttachmentParentType::Contact => "SELECT archived_at FROM contacts WHERE id = ?1",
        AttachmentParentType::Opportunity => "SELECT archived_at FROM opportunities WHERE id = ?1",
    };
    let archived: Option<Option<String>> = connection
        .query_row(sql, [parent_id], |row| row.get(0))
        .optional()?;
    let label = parent_type.as_database_value();
    match archived {
        None => Err(invalid_input(
            "parentId",
            format!("{label} \"{parent_id}\" does not exist"),
        )),
        Some(Some(_)) => Err(invalid_input(
            "parentId",
            format!("{label} \"{parent_id}\" is archived; unarchive it before attaching files"),
        )),
        Some(None) => Ok(()),
    }
}

/// Copy a file under management while hashing it, refusing anything that grows
/// past the cap mid-copy. Returns the byte count and lowercase SHA-256.
fn copy_into_management(
    source: &Path,
    destination: &Path,
) -> Result<(u64, String), ApplicationError> {
    let mut reader = std::fs::File::open(source)?;
    let mut writer = std::fs::File::create(destination)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut total: u64 = 0;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > MAX_ATTACHMENT_BYTES {
            return Err(too_large(total));
        }
        hasher.update(&buffer[..read]);
        std::io::Write::write_all(&mut writer, &buffer[..read])?;
    }
    std::io::Write::flush(&mut writer)?;
    let digest = hasher.finalize();
    Ok((
        total,
        digest.iter().map(|byte| format!("{byte:02x}")).collect(),
    ))
}

/// Best-effort removal of one attachment's directory; true when nothing is
/// left behind (including when it was already gone).
pub(crate) fn remove_managed_directory(store: &AttachmentStore, attachment_id: &str) -> bool {
    let directory = store.directory(attachment_id);
    if !directory.exists() {
        return true;
    }
    let _ = std::fs::remove_dir_all(&directory);
    !directory.exists()
}

/// True when a source path resolves inside the managed root — attaching a
/// managed file to itself would duplicate or clobber the store.
fn is_inside_managed_root(store: &AttachmentStore, source: &Path) -> bool {
    let (Ok(root), Ok(source)) = (store.absolute_root().canonicalize(), source.canonicalize())
    else {
        return false;
    };
    source.starts_with(root)
}

/// The safe on-disk name for a source file: no separators or control
/// characters, no trailing dots or spaces, bounded length, and never a Windows
/// device name. This is also the display name stored on the row, so every
/// consumer of `file_name` gets a path-safe value.
pub fn sanitized_file_name(raw: &str) -> Result<String, ApplicationError> {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or(raw);
    let filtered: String = base
        .chars()
        .filter(|character| {
            !character.is_control()
                && !matches!(
                    character,
                    '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*'
                )
        })
        .collect();
    let trimmed = filtered.trim().trim_end_matches(['.', ' ']).trim();
    if trimmed.is_empty() {
        return Err(invalid_input(
            "sourcePath",
            format!("\"{raw}\" has no usable file name"),
        ));
    }
    let capped = cap_file_name(trimmed);
    let stem = capped.split('.').next().unwrap_or("");
    if WINDOWS_RESERVED_NAMES
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(stem))
    {
        return Ok(format!("_{capped}"));
    }
    Ok(capped)
}

/// Truncate a long name on a character boundary, keeping a short extension.
fn cap_file_name(name: &str) -> String {
    if name.len() <= MAX_FILE_NAME_BYTES {
        return name.to_owned();
    }
    let extension = Path::new(name)
        .extension()
        .and_then(OsStr::to_str)
        .filter(|extension| extension.len() <= 16)
        .map(|extension| format!(".{extension}"))
        .unwrap_or_default();
    let stem = Path::new(name)
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or(name);
    let mut end = MAX_FILE_NAME_BYTES
        .saturating_sub(extension.len())
        .min(stem.len());
    while end > 0 && !stem.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{extension}", &stem[..end])
}

/// Media type from a small extension map; anything unknown is opaque bytes.
fn media_type_for(file_name: &str) -> String {
    let extension = Path::new(file_name)
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "txt" => "text/plain",
        "md" => "text/markdown",
        "csv" => "text/csv",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
    .to_owned()
}

fn invalid_input(field: &str, message: String) -> ApplicationError {
    ApplicationError::InvalidInput {
        field: field.to_owned(),
        message,
    }
}

fn too_large(size: u64) -> ApplicationError {
    ApplicationError::ValidationFailed {
        code: "file_too_large",
        field: "sourcePath".into(),
        message: format!(
            "the file is {size} bytes; attachments are limited to {MAX_ATTACHMENT_BYTES} bytes"
        ),
    }
}
