# Attachments implementation brief

Issue: #21
Status: implemented
Updated: 2026-08-18

## Boundary

Add attachments — files a user can attach to a contact or opportunity — as managed copies under
the application's own storage, tracked in a new `attachments` table (migration 010), behind the
same Rust application seam as every other write. Files are copied under management (not
referenced in place), so a moved or deleted source file never breaks an attachment. This milestone
extends the portable archive (#20) to carry the managed files themselves as real ZIP entries. It
does not add FTS indexing of attachment file names, merge-import, or attachment handling in
database backup/restore (`backup_to`/`restore_from` remain database-file only).

## Design decisions

- **Managed copies, not references.** `add_attachment` copies the source file into
  `<app data>/attachments/<attachment id>/<file name>` and hashes it while copying, rather than
  storing a path to wherever the user's file happens to live. The database row is the record of
  truth (name, media type, size, SHA-256); the file's on-disk name is the row's already-sanitized
  `file_name`, so every consumer — including the archive exporter — can build a safe path from the
  row without re-sanitizing.
- **Polymorphic parent enforced with triggers, following the `record_tags` precedent.** SQLite
  cannot express a foreign key selected by a column, so `attachments_owner_insert` /
  `attachments_owner_update` triggers reject a row whose `parent_type`/`parent_id` doesn't point at
  a live contact or opportunity, and `contacts_attachments_delete` /
  `opportunities_attachments_delete` triggers block deleting a parent that still has attachments —
  the same pattern migration 8 established for tags and custom fields.
- **Sanitized file name doubles as the on-disk name.** `sanitized_file_name` strips path
  separators and control characters, trims trailing dots/spaces, caps length at 120 bytes
  (preserving the extension), and prefixes Windows reserved device names (`CON`, `PRN`, `NUL`,
  `COM1`…, `LPT1`…) with `_` so an archive stays portable even though the app only ships for
  macOS/Windows today. The internal `relative_path` (`<id>/<file_name>`) is `UNIQUE` and never
  crosses the wire — callers resolve a file through `attachment_path`, never by building a path
  themselves.
- **256 MiB cap shared with the archive.** `MAX_ATTACHMENT_BYTES` is literally
  `archive::MAX_ENTRY_BYTES`, so nothing that can be attached is too large to also travel in a
  portable archive later. The cap is enforced twice: once against the source file's `metadata().len()`
  before any bytes are copied, and again while streaming the copy (`copy_into_management`), so a
  file that grows mid-copy is still caught rather than trusted from its initial size.
- **Copy-then-write-row, with cleanup on either failure.** `add_attachment` copies and hashes the
  file first; if the database insert then fails (e.g., a race where the parent was archived between
  the check and the write), the managed copy is removed so the filesystem never keeps a file no row
  points at. `remove_attachment` is the mirror: the row is deleted first inside the transaction (the
  row is authoritative — once it commits, the attachment is gone even if bytes can't be deleted),
  then the managed directory is removed best-effort; `AttachmentRemoval.fileRemoved` reports
  whether that cleanup actually succeeded, since a left-behind file is harmless (nothing lists or
  exports a row-less attachment).
- **Self-ingestion refused.** `add_attachment` canonicalizes both the managed root and the source
  path and refuses a `sourcePath` that already resolves inside the attachments root — attaching a
  managed file to itself would duplicate or clobber the store.
- **`attachment_path` resolves, it doesn't open.** The command returns an absolute path plus
  `exists: bool` rather than opening the file itself; the frontend hands that path to the new
  `tauri-plugin-opener` dependency to launch the OS default handler. `exists: false` is not an
  error — it's how a caller learns a managed file is missing (e.g., after restoring a database
  backup, which never touches attachment files).
- **Archive carries real files, verified two ways.** `attachments` joins `ARCHIVE_TABLES` as the
  17th canonical table, and export writes each managed file to `assets/<attachment id>/<file name>`
  reading it fresh from disk — refusing the whole export (`attachment_file_missing`) if a
  referenced file is gone, since an archive that can't be imported is worse than no archive.
  Verification checks every asset twice: once as an ordinary manifest-listed file (size + SHA-256
  against `manifest.json`, same as every other archived file), and again against the owning row's
  own `size_bytes`/`sha256` (`attachment_size_mismatch`, `attachment_checksum_mismatch`) — the row
  is what the live database will trust after import, so it gets its own check independent of
  whatever the manifest claims. A file under `assets/` with no claiming row is `unexpected_asset`.
- **Stage, commit, swap — never a half-imported managed file.** `import_archive` writes verified
  attachment bytes into a fresh `.import-staging-<id>` directory under the attachments root
  *before* the database transaction runs, so anything that can fail on the filesystem (disk full,
  permissions) fails before the live database is touched. Only after the transaction commits does
  `swap_staged_assets` clear the old managed directories and rename the staged files into place — the
  database is authoritative, so files never move ahead of the rows that describe them. A refused
  import (verification fails) never creates or touches the staging directory at all; a failure
  between staging and swap is reported but the already-committed row replace stands.
- **Forward compatibility for whole tables, not just columns.** `TABLE_INTRODUCED_IN` maps each
  canonical table to the migration that created it; a missing `data/<table>.json` is tolerated as
  zero rows only when the archive's `databaseMigrationVersion` predates that table's introduction.
  This is new with attachments and supersedes the #20-era rule that all table files were always
  required: an archive written at migration 9 (before `attachments` existed) has no
  `data/attachments.json` and still imports cleanly, with zero attachment rows and nothing under
  `assets/`. It layers on top of the existing per-column nullable-default tolerance rather than
  replacing it.
- **Media type from a small extension map.** `media_type_for` covers common contractor-relevant
  types (PDF, images, Office documents, plain text/Markdown/CSV, ZIP) and falls back to
  `application/octet-stream` for anything else — good enough for a UI icon or an opener hint,
  without pulling in a magic-byte sniffing dependency.
- **Explicitly out of scope for v1.** Attachment file names are not indexed by FTS5 search.
  Merge-import is not supported (attachments replace in full along with everything else). Database
  backup/restore is unchanged and database-file only — it never copies attachment bytes, so a
  restored database can carry `attachments` rows whose files are gone; `attachment_path` surfaces
  that as `exists: false` rather than failing.

## Contracts

`schemas/v1/local-api.json` adds four commands and their wire types (`src-tauri/src/attachments.rs`
is the single source; the schema is verified against it by `src-tauri/tests/schema_contracts.rs`):

- `add_attachment(request: AddAttachmentRequest)` — write; returns `Attachment`.
- `list_attachments(parentType, parentId)` — read; returns `Attachment[]`.
- `remove_attachment(request: RemoveAttachmentRequest)` — write; returns `AttachmentRemoval`.
- `attachment_path(attachmentId)` — read; returns `AttachmentLocation`.

Wire types: `Attachment { id, parentType, parentId, fileName, mediaType, sizeBytes, sha256,
createdAt, version }` (never exposes `relative_path`), `AddAttachmentRequest { actor?, parentType,
parentId, sourcePath }`, `RemoveAttachmentRequest { actor?, attachmentId, expectedVersion }`,
`AttachmentRemoval { fileRemoved }`, `AttachmentLocation { path, exists }`.

`schemas/v1/data-model.json` adds the `attachments` table (migration 010, indexed on
`(parent_type, parent_id, created_at)`) and its four triggers
(`attachments_owner_insert`/`_update`, `contacts_attachments_delete`,
`opportunities_attachments_delete`).

See `docs/DATA_MODEL.md` "attachments" and "Archive contract" for the on-disk layout and
`docs/LOCAL_API.md` for full command semantics.

## Verification

`src-tauri/tests/attachments.rs` (10 tests) covers:

- **Round trip.** A file is copied under management, listed, and removed; removal deletes the row
  and best-effort the file, reporting `fileRemoved`.
- **Not found.** Removing an unknown attachment id is `not_found`.
- **Sanitization.** Path separators, control characters, trailing dots/spaces, an overlong name,
  and a Windows reserved device name all land as a safe on-disk name — and that sanitized name is
  what's actually on disk, not just what the row claims.
- **Size cap.** A file over 256 MiB is refused before anything is copied.
- **Parent requirements.** Attaching needs an existing, active (non-archived) parent and a
  readable source file.
- **Self-ingestion.** A file already inside the managed root cannot be attached again.
- **`attachment_path`.** Returns an absolute path and reports `exists: false` for a file that's
  gone.
- **Migration.** `migration_010_creates_attachments_on_fresh_and_upgraded_databases` — the table,
  index, and triggers exist identically whether the database starts fresh or upgrades through
  migration 010.
- **Trigger enforcement.** The owner trigger refuses a row without a live parent.

`src-tauri/tests/portable_archive.rs` gained 5 attachment-specific tests on top of its existing 20:

- **Round trip.** An archive carrying managed attachment files replaces the target database's
  managed files along with its rows.
- **Tamper detection.** Modified attachment bytes are refused both by the manifest checksum and,
  independently, by the owning row's checksum.
- **Missing file.** An `attachments` row with no matching `assets/` entry is refused
  (`attachment_file_missing`).
- **Orphan asset.** A file under `assets/` with no claiming row is refused (`unexpected_asset`).
- **Forward compatibility.** An archive written before migration 010 (no `attachments` table, no
  `data/attachments.json`) still imports cleanly.
- **Export guard.** `export_archive` refuses to write an archive when a referenced managed file is
  missing from disk.
