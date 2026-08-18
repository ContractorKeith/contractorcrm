# Data model

Status: implemented through database migration 009
Updated: 2026-08-18

The machine-readable v1 contract is `schemas/v1/data-model.json`, including
each implemented table's columns, required fields, primary key, foreign keys,
and SQL checks.
Forward-only SQLite migrations remain the executable source of truth and are
verified field-by-field against that contract by
`src-tauri/tests/schema_contracts.rs`.

## Domain language

Contractor-facing words, not sales-team CRM jargon:

- A **contact** is a person — client, lead, prospect, sub, vendor, supplier, or site contact.
- A **company** groups contacts and can itself be a client, sub, or vendor.
- An **opportunity** is potential work moving through the pipeline toward won or lost.
- A **stage** is a user-customizable pipeline step (default: Lead → Estimating → Proposal Sent → Negotiation → Won / Lost).
- An **activity** is a logged touch — call, email, text, site visit, meeting, or note — on a contact or opportunity.
- A **task** is a follow-up or to-do with a due date, optionally linked to a record.
- A **saved view** is a named filter over contacts, companies, or opportunities.
- A **hand-off** is a versioned envelope or reference linking an opportunity to a quote or a ContractorProject job.

Use `job` only for the ContractorProject record an opportunity hands off to; the CRM never owns jobs.

## Core records

### `companies`

- `id`
- `name`
- `kind` (`client`, `lead`, `sub`, `vendor`, `supplier`, `other`)
- `phone`, `email`, `website`, address fields
- `service_area`, `license_notes` (trade-relevant custom basics)
- `notes`
- `archived_at` nullable
- `created_at`, `updated_at`, `version`

### `contacts`

- `id`
- `company_id` nullable — solo homeowners need no company
- `first_name`, `last_name`, `display_name`
- `role` (`owner`, `estimator`, `site_contact`, `office`, `other`)
- `kind` (`client`, `lead`, `sub`, `vendor`, `supplier`, `other`)
- phones and emails as typed multi-value rows (`contact_channels`: kind, label, value, preferred flag)
- `preferred_contact_method`
- address fields, `property_type`
- `notes`
- `favorite` flag
- `external_id` nullable — stable import identity, unique when set (partial index, migration 009)
- `archived_at` nullable
- `created_at`, `updated_at`, `version`

Derived, not stored as truth: `last_contacted_at` and next open task come from activities and tasks; they are replaceable projections.

CSV import matches an incoming row to an existing contact by `external_id` first, then by record `id`, so a file exported from the CRM re-imports onto the same contacts. Exports emit `COALESCE(external_id, id)` for the same reason.

### `tags` and `record_tags`

Tags have a unique case-insensitive 1–80 character label, optional `neutral`, `accent`, or `attention` color role, archive timestamp, timestamps, and a version. `record_tags` joins a tag to a contact, company, or opportunity by entity type + id. Tags are flat and archived rather than purged.

### `custom_field_defs` and `custom_field_values`

User-defined fields per entity type (`text`, `number`, `date`, `select`) have stable ids, archive timestamps, versions, and ordered stable select options. Values join one definition to one record using exactly one typed column. Polymorphic-record and definition/type/option invariants are SQLite triggers; referenced select options cannot be removed. No formulas or required fields.

### `pipelines` and `stages`

One default pipeline in v1, but stages are user-editable rows, not an enum:

- `stages`: `id`, `pipeline_id`, `name`, `sort_key`, `kind` (`open`, `won`, `lost`)

Exactly one `won` and one `lost` stage per pipeline. Renaming or reordering stages never rewrites opportunity history.

### `opportunities`

- `id`
- `name`
- `contact_id`, `company_id` (at least one required)
- `stage_id`
- `value_minor`, `currency_code` — integer minor units, no float money
- `probability_percent` optional
- `expected_close_date` optional
- `source` (`referral`, `repeat_client`, `website`, `sign`, `other` + free label)
- `lost_reason_id` nullable — required when moved to the lost stage
- `quote_ref` nullable: external tool name, external ID, label, linked timestamp
- `job_ref` nullable: same shape, pointing at a ContractorProject job once won
- `notes`
- `archived_at` nullable
- `created_at`, `updated_at`, `version`

`stage_history` records each stage change (opportunity, from/to stage, actor, timestamp) so "how long in Estimating" is answerable without mutating the opportunity row.

### `lost_reasons`

User-editable list: `id`, `label`, `sort_key`, `active` flag. Ships with sensible defaults (price, timing, went with competitor, no response, out of scope).

### `activities`

- `id`
- `parent_type` + `parent_id` — a contact, company, or opportunity
- `kind` (`call`, `email`, `text`, `site_visit`, `meeting`, `note`)
- `direction` (`inbound`, `outbound`, `none`)
- `occurred_at` — user-editable; logging yesterday's call is normal
- `summary` (short) and `body` (Markdown)
- `actor` (`user`, `agent`, `import`)
- `created_at`, `updated_at`, `version`

An activity on an opportunity also appears on the linked contact's timeline as a projection — stored once, joined at read time.

### `tasks`

- `id`
- `title`, `body`
- `parent_type` + `parent_id` nullable — personal tasks have no parent
- `due_at` nullable, `remind_at` nullable
- `priority` (`low`, `normal`, `high`)
- `status` (`open`, `done`, `dropped`), `completed_at`
- `created_at`, `updated_at`, `version`

Completing a task can optionally log a linked activity in the same transaction.

### `attachments`

A contact or opportunity attachment (issue #21, implemented in migration 010) is a copy of the
user's file taken under application management, laid out on disk as
`<app data>/attachments/<attachment id>/<file name>`. The row is the record of truth:

- `id`
- `parent_type` + `parent_id` — contact or opportunity, enforced by trigger (SQLite cannot express
  a foreign key selected by a column, the same `record_tags`-style pattern as migration 8); the
  parent must exist and not be archived, and a parent cannot be deleted while it still has
  attachments
- `file_name` — sanitized display name, which is also the on-disk file name (traversal/control
  characters and invisible Unicode format characters — bidi overrides, zero-width spaces, variation
  selectors — stripped so a right-to-left override can't disguise an executable as a PDF, Windows
  reserved device names prefixed with `_`, length capped at 120 bytes with the extension preserved)
- `relative_path` — internal, `UNIQUE`, `<id>/<file_name>`; never exposed over the wire, only used
  to resolve the managed file path
- `media_type` nullable — looked up from a small file-extension map, `application/octet-stream`
  for anything unrecognized
- `size_bytes`, `sha256` — recorded when the file is copied into management
- `created_at`, `version`

Managed files are capped at 256 MiB each — the same per-entry cap the portable archive already
enforces, so anything that can be attached can also be exported. File content lives outside SQLite,
under the attachments root; the database backup/restore commands are database-file only and never
touch attachment files (see "Archive contract" below for how attachments travel in a portable
archive, and how a restored database can end up with rows that reference missing files).

Four commands cover the surface: `add_attachment` (copies a file from `sourcePath` under
management; refuses a `sourcePath` that already resolves inside the managed root, so a managed
file can't be attached to itself), `list_attachments`, `remove_attachment` (versioned; deletes the
row first, then best-effort removes the managed file — `fileRemoved` reports whether that cleanup
succeeded), and `attachment_path` (resolves the absolute path plus whether the file still exists on
disk, for the frontend to hand to the `tauri-plugin-opener` opener rather than building a path
itself). FTS indexing of attachment file names and merge-import are both out of scope for v1.

### `saved_views`

`id`, `name`, `entity_type`, `definition_json`, `sort_key`, timestamps, and optimistic `version`.

The current definition is strict JSON with `schemaVersion: 2`,
`filter.includeArchived`, bounded `filter.tagIdsAll`, finite typed
`filter.customFields` predicates, and `sort` (`field` plus `ascending` or
`descending`). Tag and custom-field predicates use AND semantics; allowed
operators are defined by field type and never accept caller-defined SQL,
columns, or operators. Views apply only to contact, company, and opportunity
lists. Sort fields are bounded by surface: `displayName`; `name`; or
`name`/`stage`/`value`/`expectedClose` respectively. Known unversioned v0 and
v1 definitions migrate to v2 in memory on read without rewriting stored bytes;
unsupported future or malformed JSON is rejected. Names are unique
case-insensitively per surface; each surface is capped at 50 views and ordered
by `sort_key`, then id.

### `recents` and needs-attention

Recents are an app-settings projection, not domain data. Needs-attention is computed by deterministic rules (stale threshold per kind, overdue tasks, proposal-sent-no-response) over activities, tasks, and stage history — configurable thresholds live in `app_settings`, results are never stored.

## Persistence support

Same as ContractorProject:

- `schema_migrations` — forward-only migrations
- `command_log` — command ID, actor (`user`, `agent`, `import`), timestamp, bounded summary for undo/audit
- `app_settings` — non-secret preferences (thresholds, density, theme)
- FTS5 index over active contacts, companies, opportunities, and activity summaries/bodies, maintained by the repository layer inside the same transaction as the write. It is a rebuildable projection; archived records and deleted activities are removed immediately. Contact channel values are included.
- Provider credentials never stored in these tables

## Invariants

- Every child record belongs to exactly one parent record; polymorphic parents are constrained to known types.
- All application writes include an expected record version where concurrent or agent edits could overwrite newer work.
- Money uses integers plus an ISO currency code; no floating-point currency values.
- Deletes are recoverable archives unless the user explicitly purges data.
- An opportunity in the lost stage has a lost reason; one in the won stage may have a job reference.
- Stage history is append-only.
- Derived fields (last contacted, needs attention, opportunity counts) are reproducible from canonical inputs.
- Imports use stable external IDs or an explicit mapping table so retries do not duplicate records.

## Archive contract

The portable archive (issue #20, extended with attachments in issue #21) is a versioned ZIP:

- `manifest.json` — `schemaVersion` (currently `1`), `product` (name +
  app version), `exportedAt`, `databaseMigrationVersion`, one
  `ArchiveFileEntry` (`path`, `sha256`, `bytes`) per archived file, and
  `recordCounts` per table.
- `data/<table>.json` — one pretty-printed JSON array per canonical table, in
  camelCase, for all 17 archived tables: `companies`, `contacts`,
  `contact_channels`, `pipelines`, `stages`, `lost_reasons`, `opportunities`,
  `stage_history`, `activities`, `tasks`, `saved_views`, `tags`,
  `record_tags`, `custom_field_defs`, `custom_field_options`,
  `custom_field_values`, `attachments`. `command_log`, `app_settings`,
  `search_index`, and `schema_migrations` are deliberately excluded —
  history/preferences are local, the FTS index is rebuilt on import, and
  migrations belong to the database, not the archive.
- `csv/contacts.csv` and `csv/opportunities.csv` — human-readable convenience
  copies of the CSV export; `import_archive` ignores them.
- `assets/<attachment id>/<file name>` — the managed attachment files
  themselves, one per `attachments` row, at the same relative path they hold
  on disk under the attachments root. Export reads each managed file fresh
  and refuses (`attachment_file_missing`) if one is gone, so an archive can
  never claim a file it doesn't actually carry. The directory entry is always
  present, even with zero attachments, so the layout is stable.

Export enforces the same ~1 GiB total-uncompressed cap import reads against (`MAX_ARCHIVE_BYTES`):
before building the ZIP, `export_archive` sums the archived table JSON, the CSV convenience copies,
and every attachment's `size_bytes` (from its row, not a re-read of the file) as it goes, and
refuses (`validation_failed` / `archive_too_large`) the moment the running total would exceed the
limit — an archive too large to ever be imported back is refused at export time instead of being
written and only failing later.

Attachment files are cross-checked against their row on top of the manifest's own per-file
checksum: every `attachments` row must have a matching `assets/<id>/<file_name>` entry
(`attachment_file_missing`), whose byte count and SHA-256 must match the row's `size_bytes` and
`sha256` (`attachment_size_mismatch`, `attachment_checksum_mismatch`); any file under `assets/`
that no row claims is `unexpected_asset`. Only once every attachment file verifies clean are its
bytes handed on to import.

On import, verified attachment bytes are written into a fresh `.import-staging-<id>` directory
under the attachments root before the database transaction runs, then swapped into place after the
transaction commits — see "A successful import..." below for the full stage/commit/swap/sweep
sequence and its honest recovery story (there is no directory-swap recovery; a stranded staging
directory is only ever swept and discarded, never resumed).

Archive schema version and database migration version are tracked
independently. Verification (shared by `preview_archive_import` and
`import_archive`, so a preview with no issues really does mean import will
succeed) never writes to the live database and runs, in order:

1. **Untrusted-input bounds.** Each entry is read through a 256 MiB per-entry
   cap and a 1 GiB total-uncompressed cap (`entry_too_large`,
   `archive_too_large`); an aborted read still spends its budget, so a
   compression-ratio bomb can't buy unbounded work. Entry-path validation
   rejects absolute paths, backslashes, `.`/`..` traversal, and unknown files
   (`entry_path_absolute`, `entry_path_backslash`, `entry_path_traversal`,
   `unknown_file`), and a duplicate entry the ZIP central directory collapsed
   is caught by comparing declared vs. unique entry counts
   (`duplicate_entry`). Issue reporting stops at 100 problems, with a
   trailing `too_many_issues` summarizing how many more were found.
2. **Product and version gates.** An archive from a different product
   (`wrong_product`) short-circuits the rest of verification — one clear
   problem instead of seventeen missing table files. Otherwise
   `schemaVersion == 1` (`unsupported_schema_version`) and
   `databaseMigrationVersion <= supported` (`unsupported_migration_version`;
   an older archive imports forward, a newer one is rejected until the app is
   updated) are checked next.
3. **Checksums and row shape.** Every manifest-listed file must be present
   with a matching size and SHA-256 (`missing_file`, `size_mismatch`,
   `checksum_mismatch`), and every entry in the archive must be listed
   (`unlisted_file`). A missing `data/<table>.json` is a `missing_table_file`
   issue *unless* the archive's `databaseMigrationVersion` predates the
   migration that introduced the table (`TABLE_INTRODUCED_IN`) — an archive
   written before migration 10, for example, has no `attachments` file and
   still imports cleanly, treated as zero attachment rows. This supersedes
   the earlier "all 16 table files are always required" rule from issue #20;
   it is forward-compatibility for whole tables, layered on top of the
   existing per-column tolerance below. Every row is checked against the
   live schema read via `PRAGMA table_info` (unknown/missing columns,
   type/nullability mismatches, blank ids, invalid versions, duplicate
   primary keys — `record_tags` keys on `(tag_id, entity_type, record_id)`,
   every other table on `id`). A column *missing* from an older archive's
   row is allowed only when the live column is nullable (defaults to
   `NULL`). `app_settings`/needs-attention thresholds never travel in the
   archive at all, since that table is excluded entirely.
   Attachment files under `assets/` are then cross-checked against the
   `attachments` rows. Each row's `id` and `file_name` must be safe,
   validated path segments and its `relative_path` must equal
   `<id>/<file_name>` (`invalid_value` for an unsafe id or file name,
   `attachment_path_mismatch` for a `relative_path` that doesn't match) —
   a hostile archive can never plant a row that addresses anything outside
   the attachments root. Only once a row passes that check is its asset file
   verified (`attachment_file_missing`, `attachment_size_mismatch`,
   `attachment_checksum_mismatch`, `unexpected_asset` — see "Archive
   contract" above).
4. **Record counts.** The manifest's claimed `recordCounts` are compared
   against the rows actually parsed per table (`record_count_mismatch`) —
   an inflated manifest count over an emptied data file would otherwise
   import as a silent wipe. `preview_archive_import` reports the *parsed*
   counts, not the manifest's claim.
5. **References and structure** (only once every row parsed cleanly —
   referential and structural checks are skipped when earlier issues exist,
   so fixing those issues and re-previewing the file can surface new ones).
   Referential integrity is checked in memory across all 17 tables,
   including polymorphic `parent_type`/`parent_id` and
   `entity_type`/`record_id` ownership (`missing_reference`,
   `unknown_parent_type`). Structurally, the archive must contain at least
   one pipeline with stages covering the `open`, `won`, and `lost` kinds —
   the app can't function without one (`missing_pipeline`,
   `missing_stage_kind`).
6. **Dry-run apply.** Only once every earlier check passes, verification
   applies the full delete-and-replace into a throwaway in-memory database
   (never committed) — exercising every `UNIQUE`/`CHECK` constraint, trigger,
   and the search-index rebuild — and reports any failure as
   `constraint_violation`. This is what makes "empty issues" true by
   construction rather than by hope.

A successful import sweeps any `.import-staging-*` directory a previous crashed import left
behind, stages the verified attachment files into a fresh staging directory under the attachments
root, then takes its timestamped safety backup (`<database>.pre-import-<stamp>.bak`) only after
verification passes with no issues, then replaces every canonical row — delete all 17 tables in
reverse dependency order, insert from the archive in dependency order, rebuild the FTS index — in
one transaction. If the real row-replace apply somehow fails despite the dry run having passed,
the transaction rolls back, the orphaned safety backup and staging directory are removed, and the
failure is reported as `validation_failed` / `archive_invalid` — the live database and filesystem
are left exactly as they were. Only full replace is supported in v1; merge-import is out of scope.

Once the transaction commits, the import is committed and done as far as the database is
concerned — the row replace is irreversible from here, whatever happens next. The swap from the
staging directory into the live attachments root (clear the old managed files, move the staged
ones in) is attempted, retried once on failure, and then abandoned silently if it still fails:
`import_archive` always returns its `ArchiveImportReport` (with the safety backup path) rather
than surfacing a post-commit filesystem error the caller can't undo anyway. A swap that never
completes leaves some attachment rows pointing at files that aren't there yet; `attachment_path`
reports those as `exists: false` until the archive is imported again, which re-stages and
re-attempts the swap from scratch. Export and import each write one `command_log` row
(`export`/`archive` and `import`/`archive`).

`export_archive` also sweeps any leftover `.import-staging-*` directory before it reads the
attachments root, since export is the other routine opportunity (besides the next import) to clear
bytes a crashed import stranded. `export_archive` (and the CSV exports) refuse to write onto the
live database file itself or its `-wal`/`-shm` sidecars and `.bak` safety copies
(`destination_is_database`), so an export can never overwrite the data it was meant to preserve.

Backup/restore (`backup_to` / `restore_from`) is a separate, database-file-only mechanism — it
never copies or restores attachment files. Restoring a database backup can leave `attachments`
rows that reference files no longer on disk; `attachment_path` still resolves the row's path but
reports `exists: false`, so the frontend can surface the gap rather than fail silently. The
portable archive is the only mechanism that carries attachment bytes.
