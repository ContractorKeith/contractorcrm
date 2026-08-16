# Data model

Status: implemented through database migration 006
Updated: 2026-08-16

The machine-readable v1 contract is `schemas/v1/data-model.json`. Forward-only
SQLite migrations remain the executable source of truth and are verified
against that manifest by `src-tauri/tests/schema_contracts.rs`.

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
- `archived_at` nullable
- `created_at`, `updated_at`, `version`

Derived, not stored as truth: `last_contacted_at` and next open task come from activities and tasks; they are replaceable projections.

### `tags` and `record_tags`

Tags have an id, label, and optional color role. `record_tags` joins a tag to a contact, company, or opportunity by entity type + id. Tags are flat in v1 — no hierarchies.

### `custom_field_defs` and `custom_field_values`

User-defined fields per entity type (`text`, `number`, `date`, `select`) with ordered options for selects. Values join a definition to a record. Keep v1 simple: no formulas, no required-field enforcement.

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

`id`, `name`, `entity_type`, versioned filter definition (JSON), `sort_key`. Filter schema versions are forward-migratable like the database.

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

The portable archive is a versioned ZIP mirroring ContractorProject's:

- `manifest.json` with archive version, product version, and checksums
- canonical CRM data as JSON (companies, contacts, opportunities, stages, activities, tasks, tags, custom fields, saved views)
- attachments under a confined `assets/` directory
- optional human-readable CSV exports (contacts, opportunities)

Import validates paths, checksums, schema version, IDs, and referential integrity before writing anything. The whole import is transactional. CSV contact import ships in v1 with a mapping preview — every contractor arrives with a spreadsheet.
