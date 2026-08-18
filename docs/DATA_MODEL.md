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

Metadata plus a managed relative path under the app assets directory: parent type + id, filename, media type, size, checksum. File content stays outside SQLite and inside the portable archive.

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

The portable archive (issue #20, implemented) is a versioned ZIP:

- `manifest.json` — `schemaVersion` (currently `1`), `product` (name +
  app version), `exportedAt`, `databaseMigrationVersion`, one
  `ArchiveFileEntry` (`path`, `sha256`, `bytes`) per archived file, and
  `recordCounts` per table.
- `data/<table>.json` — one pretty-printed JSON array per canonical table, in
  camelCase, for all 16 archived tables: `companies`, `contacts`,
  `contact_channels`, `pipelines`, `stages`, `lost_reasons`, `opportunities`,
  `stage_history`, `activities`, `tasks`, `saved_views`, `tags`,
  `record_tags`, `custom_field_defs`, `custom_field_options`,
  `custom_field_values`. `command_log`, `app_settings`, `search_index`, and
  `schema_migrations` are deliberately excluded — history/preferences are
  local, the FTS index is rebuilt on import, and migrations belong to the
  database, not the archive.
- `csv/contacts.csv` and `csv/opportunities.csv` — human-readable convenience
  copies of the CSV export; `import_archive` ignores them.
- `assets/` — a directory entry reserved for attachments; empty until
  issue #21 ships managed files.

Archive schema version and database migration version are tracked
independently. `import_archive` verifies the whole archive before writing
anything: entry-path validation (no absolute paths, backslashes, `.`/`..`
traversal, unknown files, or a duplicate entry the central directory
collapsed), per-file size and SHA-256 checksum against the manifest,
`schemaVersion == 1`, and `databaseMigrationVersion <= supported` (an older
archive imports forward; a newer one is rejected until the app is updated).
Every row is then checked against the live schema read via `PRAGMA
table_info` (unknown/missing columns, type/nullability mismatches, blank
ids, invalid versions, duplicate primary keys — `record_tags` keys on
`(tag_id, entity_type, record_id)`, every other table on `id`) and
referential integrity is checked in memory across all 16 tables, including
polymorphic `parent_type`/`parent_id` and `entity_type`/`record_id`
ownership. A column missing from an older archive is allowed when it is
nullable, so forward-compatible archives import cleanly. Any issue reported
by `preview_archive_import` blocks `import_archive`.

A successful import takes a timestamped safety backup first
(`<database>.pre-import-<stamp>.bak`), then replaces every canonical row —
delete all 16 tables in reverse dependency order, insert from the archive in
dependency order, rebuild the FTS index — in one transaction, so a failure
leaves the live database untouched. Only full replace is supported in v1;
merge-import is out of scope. Export and import each write one
`command_log` row (`export`/`archive` and `import`/`archive`).
