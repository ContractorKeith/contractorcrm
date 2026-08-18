# Portable archive implementation brief

Issue: #20
Status: implemented
Updated: 2026-08-18

## Boundary

Add versioned portable archive export/import — a single ZIP file carrying
every canonical CRM record — as Rust application commands behind the same
seam as every other write. This milestone does not add attachments (#21;
`assets/` ships as an empty, reserved directory), merge-import (only full
replace is supported), or any UI beyond what's needed to call the commands
through a native file dialog.

## Design decisions

- **ZIP + manifest + SHA-256, not a bespoke format.** The `zip` crate (Deflate
  compression only) and `sha2` are the two new dependencies. A ZIP is
  inspectable with any archive tool, and per-file checksums plus recorded
  byte counts in `manifest.json` let import detect truncation or tampering
  without touching the database first.
- **Verify-then-replace, never write incrementally.** `verify_archive` (the
  shared core of `preview_archive_import` and `import_archive`) reads the
  whole archive into memory, validates every entry path, checksum, manifest
  version, row shape, and cross-table reference, and only then does
  `import_archive` touch SQLite — in one transaction. A caller can preview an
  archive with zero risk, and an invalid archive never leaves the database
  partially overwritten.
- **PRAGMA-driven row validation for forward compatibility.** `table_columns`
  reads column name, declared SQLite type, and `NOT NULL` straight from
  `PRAGMA table_info(<table>)` instead of a hand-maintained column list, so
  the archive format can never drift from the live migrations. A column
  missing from an older archive's row is accepted (defaults to `NULL`) as
  long as the live schema allows null there — the mechanism that lets an
  older archive import forward into a newer schema version without a special
  migration path for archives.
- **`databaseMigrationVersion` gates direction, not equality.** An archive
  written at an older migration version imports forward (missing nullable
  columns are simply absent from its JSON rows); an archive written at a
  newer version than the running app supports is rejected with
  `unsupported_migration_version` so the user updates the app first, rather
  than importing rows the schema doesn't understand. `schemaVersion` (the
  ZIP/manifest container format, currently `1`) is versioned independently of
  both `databaseMigrationVersion` and the hand-off envelope / MCP API
  versions (see `docs/LOCAL_API.md`).
- **Stable machine-readable issue codes.** Every problem `verify_archive`
  finds is an `ArchiveIssue { code, message }` rather than a bail-on-first-
  error: `entry_path_backslash`, `entry_path_absolute`, `entry_path_traversal`,
  `unknown_file`, `duplicate_entry`, `missing_file`, `size_mismatch`,
  `checksum_mismatch`, `unlisted_file`, `unsupported_schema_version`,
  `unsupported_migration_version`, `missing_table_file`, `invalid_table_json`,
  `unknown_column`, `missing_column`, `invalid_value`, `invalid_id`,
  `invalid_version`, `duplicate_primary_key`, `missing_reference`,
  `unknown_parent_type`. `preview_archive_import` returns the full list;
  `import_archive` fails with the first issue's message (plus a count of any
  more) so a caller doesn't need two round trips to know whether an archive
  is safe to import.
- **Full replace, not merge, for v1.** Import deletes all 16 canonical tables
  (reverse dependency order) and re-inserts every archived row (dependency
  order) inside one transaction, then rebuilds the FTS index. This is the
  simplest semantics that's still safe: a merge/upsert strategy raises
  conflict-resolution questions (which side wins on a diverging edit) that
  are explicitly deferred rather than guessed at.
- **Mandatory pre-import safety backup.** `import_archive` calls
  `Storage::safety_copy("pre-import")` — the same mechanism used before
  destructive migrations — before opening the write transaction, producing
  `<database>.pre-import-<stamp>.bak`. Because the replace is transactional,
  the backup is a belt-and-suspenders recovery path for "I imported the wrong
  file," not a correctness requirement.
- **In-memory referential integrity, not SQLite foreign keys.** Plain foreign
  keys (`ARCHIVE_FOREIGN_KEYS`, 14 pairs) and polymorphic parent/id pairs
  (`ARCHIVE_POLYMORPHIC_KEYS`, 4 pairs covering `activities`, `tasks`,
  `record_tags`, `custom_field_values`) are checked against the archive's own
  row IDs before any write, since SQLite can't express polymorphic
  ownership as a `FOREIGN KEY` and checking in-memory lets every violation be
  reported at once instead of failing mid-transaction on the first bad row.
- **Excluded tables.** `command_log`, `app_settings`, `search_index`, and
  `schema_migrations` are never archived: command history and preferences
  are local to a machine, the FTS index is a rebuildable projection (import
  calls `rebuild_search_index` after the replace), and migrations belong to
  the database's own lifecycle, not a portable snapshot.
- **CSV copies are convenience-only.** `csv/contacts.csv` and
  `csv/opportunities.csv` reuse the existing `write_contacts_csv` /
  `write_opportunities_csv` helpers from the CSV import/export feature so a
  contractor can eyeball an archive's contents in a spreadsheet; import
  ignores both files entirely (`entry_path_issue` allows anything under
  `csv/` without listing it in the manifest or requiring a matching
  `data/*.json`).
- **`assets/` is a placeholder.** Export writes an empty `assets/` directory
  entry; import allows (and ignores) anything under it. Attachments (#21)
  will populate it without requiring a new archive schema version.

## Contracts

`schemas/v1/local-api.json` adds three commands and their wire types
(`src-tauri/src/archive.rs` is the single source; the schema is verified
against it by `src-tauri/tests/schema_contracts.rs`):

- `export_archive(path, overwrite)` — write; `ArchiveExportReport { path,
  recordCounts, fileCount }`.
- `preview_archive_import(path)` — read; `ArchiveImportPreview {
  schemaVersion, product, exportedAt, databaseMigrationVersion, recordCounts,
  issues: ArchiveIssue[] }`.
- `import_archive(path)` — write; `ArchiveImportReport { recordCounts,
  safetyBackupPath }`.

Supporting wire types: `ProductInfo`, `ArchiveFileEntry { path, sha256,
bytes }`, `ArchiveManifest { schemaVersion, product, exportedAt,
databaseMigrationVersion, files, recordCounts }`, `ArchiveIssue { code,
message }`, `ArchiveRecordCounts` (map of table name to row count).

See `docs/DATA_MODEL.md` "Archive contract" for the on-disk layout and
`docs/LOCAL_API.md` for full command semantics.

## Verification

`src-tauri/tests/portable_archive.rs` (10 tests) covers:

- **Round trip.** Exporting a database populated across all 16 tables
  (archived and active records, channels, stage history, activities on all
  three parent types, personal and parented tasks, saved views, tags, and all
  four custom field types) and importing into a fresh database reproduces
  every row.
- **Export guardrails.** `export_archive` reports accurate record counts and
  refuses an existing destination without `overwrite: true`.
- **Tamper detection.** A modified archive is reported by
  `preview_archive_import` and refused by `import_archive`.
- **Path safety.** Entries that escape the archive root (traversal,
  absolute paths, backslashes) are rejected.
- **Version gates.** Unsupported `schemaVersion` and
  `databaseMigrationVersion` values are rejected.
- **Referential integrity.** A dangling reference is rejected before
  anything is written.
- **Row shape.** Malformed rows are reported with stable issue codes.
- **Preview.** A clean archive is summarized with an empty `issues` list.
- **Safety backup.** Import leaves a restorable pre-import backup.
- **Caller errors.** A file that isn't an archive at all is reported as a
  caller (`invalid_input`) error, not a per-record issue.
