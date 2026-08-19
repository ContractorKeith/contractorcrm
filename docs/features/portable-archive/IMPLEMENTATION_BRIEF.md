# Portable archive implementation brief

Issue: #20
Status: implemented
Updated: 2026-08-18 (review round: dry-run verification, parsed record
counts, untrusted-input bounds, destination guards, structural minimum;
noted superseded-by-#21 table-file forward compatibility)

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
- **Verify-then-replace, ending in a dry-run apply (review-round fix).**
  `verify_archive` (the shared core of `preview_archive_import` and
  `import_archive`) is a pipeline: untrusted-input bounds and entry paths,
  product/version gates, checksums and row shape, parsed-vs-manifest record
  counts, referential integrity and structural minimums, and finally a
  **dry-run apply of the full delete-and-replace into a throwaway in-memory
  database** (`Storage::open_in_memory`, never committed). The dry run
  exercises every `UNIQUE`/`CHECK` constraint, trigger, and the search-index
  rebuild, reporting any failure as `constraint_violation`. This closed a
  real gap in the original design: "no issues reported" used to be a
  best-effort claim (duplicate tag labels or a negative custom-field amount
  could still fail mid-transaction during a real import); now it's true by
  construction, because the same replace already ran once, safely, before
  `import_archive` is allowed to touch the live database. Referential and
  structural checks — and the dry run — are skipped once an earlier stage
  already found a problem, so a caller who fixes one round of issues and
  re-previews the same file can see new issues that were hidden behind the
  first ones; this is a deliberate tradeoff (bounded work per preview) over
  running every check unconditionally.
- **Parsed record counts, not the manifest's claim (review-round fix).**
  `preview_archive_import` returns the counts of rows actually parsed from
  `data/<table>.json`, and `verify_record_counts` cross-checks those against
  `manifest.recordCounts` (`record_count_mismatch`). Trusting the manifest's
  claimed count let an attacker (or a hand-edited archive) pair an inflated
  count with an emptied data file and have it summarized as a full archive.
- **PRAGMA-driven row validation for forward compatibility — narrowly, at the
  time.** `table_columns` reads column name, declared SQLite type, and
  `NOT NULL` straight from `PRAGMA table_info(<table>)` instead of a
  hand-maintained column list, so the archive format can never drift from the
  live migrations. A column *missing from a row* in an older archive is
  accepted (defaults to `NULL`) only when the live column is nullable — that
  was, at this milestone, the entire forward-compatibility mechanism: it did
  not extend to a missing *table file* (all 16 `data/<table>.json` files were
  required, so an archive written before a table existed did not import,
  `missing_table_file`), and it does not extend to non-archived state —
  `app_settings` and the needs-attention thresholds it holds never travel in
  an archive at all, because that table is excluded from the archive
  entirely. **Superseded by issue #21.** Attachments (migration 010)
  introduced `TABLE_INTRODUCED_IN`, a map from each canonical table to the
  migration that created it, and a missing `data/<table>.json` is now
  tolerated as zero rows whenever the archive's `databaseMigrationVersion`
  predates that table's introduction — an archive written at migration 9, for
  example, has no `data/attachments.json` and still imports cleanly. See
  `docs/DATA_MODEL.md` "Archive contract" and
  `docs/features/attachments/IMPLEMENTATION_BRIEF.md` for the current rule;
  the "all 16 table files are always required" wording above is historical,
  describing the state as of this brief's own milestone.
- **`databaseMigrationVersion` gates direction, not equality.** An archive
  written at an older migration version imports forward (missing nullable
  columns are simply absent from its JSON rows); an archive written at a
  newer version than the running app supports is rejected with
  `unsupported_migration_version` so the user updates the app first, rather
  than importing rows the schema doesn't understand. `schemaVersion` (the
  ZIP/manifest container format, currently `1`) is versioned independently of
  both `databaseMigrationVersion` and the hand-off envelope / MCP API
  versions (see `docs/LOCAL_API.md`).
- **Untrusted-input bounds (review-round addition).** An archive is
  attacker- or corruption-controlled input before it's verified, so
  `read_entries` reads every entry through a 256 MiB per-entry cap
  (`MAX_ENTRY_BYTES`) and a 1 GiB total-uncompressed cap
  (`MAX_ARCHIVE_BYTES`, `entry_too_large` / `archive_too_large`) — reading
  through a `Read::take` limit rather than trusting the ZIP's declared size,
  since Deflate hides its true expansion ratio until decompressed and a
  compression bomb only lies once decompressed. An aborted read still
  charges its bytes to the running total, so a pile of oversized entries
  can't dodge the cap by each being individually rejected. `IssueLog` caps
  reported issues at 100 (`MAX_ISSUES`) with a trailing `too_many_issues`
  summary of how many more were found, so a pathological archive can't turn
  into a pathological IPC response.
- **Structural minimum (review-round addition).** `verify_structure`
  requires the archive to contain at least one pipeline whose stages cover
  the `open`, `won`, and `lost` kinds (`missing_pipeline` when there are no
  pipelines or a pipeline has no stages, `missing_stage_kind` when no
  pipeline covers all three kinds) — without that, the imported database
  can't create an opportunity at all, so it's checked as a precondition
  rather than discovered later as a broken UI.
- **Export destination guard extended to the live database family
  (review-round addition).** `check_export_destination` (shared by
  `export_archive` and both CSV exports) now also rejects a `path` that
  resolves to the live database file, its `-wal`/`-shm` sidecars, or any
  `<database>.*.bak` safety copy (`destination_is_database`), on top of the
  existing `destination_exists` guard. Without it, a user picking the wrong
  save location could silently overwrite the very data being exported.
- **Wrong-product short-circuit.** An archive whose `manifest.product.name`
  isn't `ContractorCRM` reports one `wrong_product` issue and skips the rest
  of verification, rather than reporting sixteen `missing_table_file` issues
  for a foreign archive that was never going to have this app's tables.
- **`assets/*` files refused, not silently ignored (review-round change).**
  Any entry under `assets/` is rejected outright (`unexpected_asset`) rather
  than allowed-and-ignored as originally shipped — an unread, unchecksummed
  area has no business inside a verified archive before issue #21 defines a
  bounded, checksummed attachment format. The empty `assets/` directory
  entry itself is still written on export and accepted on import as a
  placeholder.
- **Stable machine-readable issue codes.** Every problem `verify_archive`
  finds is an `ArchiveIssue { code, message }` rather than a bail-on-first-
  error — roughly 30 codes as of the review round:
  `entry_path_backslash`, `entry_path_absolute`, `entry_path_traversal`,
  `unknown_file`, `unexpected_asset`, `entry_too_large`, `archive_too_large`,
  `duplicate_entry`, `wrong_product`, `unsupported_schema_version`,
  `unsupported_migration_version`, `missing_file`, `size_mismatch`,
  `checksum_mismatch`, `unlisted_file`, `missing_table_file`,
  `invalid_table_json`, `unknown_column`, `missing_column`, `invalid_value`,
  `invalid_id`, `invalid_version`, `duplicate_primary_key`,
  `record_count_mismatch`, `missing_reference`, `unknown_parent_type`,
  `missing_pipeline`, `missing_stage_kind`, `constraint_violation`,
  `too_many_issues`. `preview_archive_import` returns the full (capped)
  list; `import_archive` fails with the first issue's message (plus a count
  of any more) so a caller doesn't need two round trips to know whether an
  archive is safe to import.
- **Full replace, not merge, for v1.** Import deletes all 16 canonical tables
  (reverse dependency order) and re-inserts every archived row (dependency
  order) inside one transaction, then rebuilds the FTS index. This is the
  simplest semantics that's still safe: a merge/upsert strategy raises
  conflict-resolution questions (which side wins on a diverging edit) that
  are explicitly deferred rather than guessed at.
- **Safety backup taken only after verification passes, removed again on
  failure (review-round refinement).** `import_archive` calls
  `Storage::safety_copy("pre-import")` — the same mechanism used before
  destructive migrations, producing `<database>.pre-import-<stamp>.bak` —
  only after `verify_archive` (including the dry run) reports zero issues;
  a rejected archive never touches the filesystem at all, not even to make a
  backup. If the real apply then somehow still fails (an unexpected error
  the dry run didn't catch), the transaction is dropped and the orphaned
  backup file is deleted before the `archive_invalid` error is returned, so
  a refused import never leaves stray `.bak` files behind. Because the
  replace is transactional and dry-run-verified, the backup is a
  belt-and-suspenders recovery path for "I imported the wrong file," not a
  correctness requirement.
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
- **`assets/` is a placeholder, not yet a carrier.** Export writes an empty
  `assets/` directory entry; import accepts that empty entry but refuses any
  file under it (`unexpected_asset` — see above). Attachments (#21) will
  populate `assets/` with actual files without requiring a new archive
  schema version.

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

`src-tauri/tests/portable_archive.rs` (20 tests) covers:

Original coverage:

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

Review-round coverage (count, constraint, size, and destination guards):

- **Manifest-vs-parsed counts.** An inflated manifest count over an emptied
  `data/<table>.json` is caught by `record_count_mismatch` in preview rather
  than importing as a silent wipe.
- **Dry-run constraint catch.** Duplicate tag labels and a negative
  custom-field amount are caught by the preview dry run
  (`constraint_violation`) instead of surfacing only during a real import.
- **Untrusted-size limits.** An oversized entry is refused
  (`entry_too_large`) without being buffered into memory — proving the
  per-entry cap is enforced during the read, not after.
- **Attachments refused.** A file under `assets/` is refused
  (`unexpected_asset`) rather than silently ignored.
- **Issue cap.** A pathologically invalid archive's issue list stops at 100
  with a trailing `too_many_issues` summary instead of growing unbounded.
- **Structural minimum.** An archive with no pipeline, or none whose stages
  cover `open`/`won`/`lost`, is refused (`missing_pipeline` /
  `missing_stage_kind`).
- **Foreign product.** An archive from a different product is refused with
  one clear `wrong_product` issue rather than sixteen missing-table-file
  issues.
- **No orphan backups.** A refused import (verification fails) leaves no
  `.pre-import-*.bak` file behind.
- **Destination guard.** `export_archive` and the CSV exports refuse to
  write onto the live database file or its `-wal`/`-shm`/`.bak` siblings
  (`destination_is_database`).
- **Export atomicity.** A failed CSV export leaves the previous file at that
  path untouched rather than truncating or corrupting it.
