# CSV import/export implementation brief

Issue: #19
Status: implemented
Updated: 2026-08-18

## Boundary

Add CSV contact import with a mapping preview and stable external ids, plus
CSV export for contacts and opportunities. Import/export are Rust application
commands behind the same seam as every other write; the UI drives them through
a native file dialog. This milestone does not add portable archive
export/import (#20), attachments (#21), company/opportunity import, or import
of custom field values.

## Design decisions

- **Stable external ids.** `contacts.external_id` (migration 009) is a
  nullable, uniquely-indexed column so hand-entered contacts stay untouched
  while imported ones carry a durable identity independent of the local row
  id. Import matches a row to an existing contact by `external_id` first,
  then falls back to matching the row's id column against a contact's local
  `id` — so a file the app exported round-trips onto the same records without
  duplicating them. Export emits `COALESCE(external_id, id)`, giving every
  exported row a usable identity even for contacts that were never imported.
- **`csv` crate.** Used for RFC 4180-correct reading and writing (quoting,
  embedded commas/newlines) instead of hand-rolled parsing.
- **`tauri-plugin-dialog` (+ `@tauri-apps/plugin-dialog`).** Native open/save
  file dialogs for choosing the import source and export destination, matching
  the desktop-native conventions used elsewhere in the app.
- **One transaction, skip-and-report semantics.** `import_contacts` parses and
  validates every row first; rows that fail validation are skipped and
  reported as `{line, reason}` without blocking the rows that are valid. All
  writes for the valid rows happen in a single immediate transaction, so a
  hard failure mid-import (e.g. a forced error) leaves the database exactly as
  it was — never a partially-applied file.
- **Company matching.** A row's `company` cell is matched to an existing
  company by case-insensitive, trimmed name; if none matches, the company is
  created (kind defaults to `client`) and logged.
- **Tags are additive.** Import never removes a contact's existing tags;
  labels from the CSV `tags` column (semicolon-separated) are added, creating
  new tags as needed, subject to the existing tag cap.
- **Channels replace only mapped kinds.** Updating a matched contact replaces
  `email` and/or `phone` channels only for the kinds actually present in the
  mapping/row; channel kinds the file doesn't carry are left alone.
- **Custom fields are export-only in v1.** Exports add one column per active
  custom field definition (rendered as text/number/date/option label); import
  does not read or write custom field values. Deferred rather than dropped —
  revisit if a future issue asks for it.
- **Preview never writes.** `preview_contact_import` parses headers and up to
  50 sample rows, returns the effective mapping (caller-supplied or
  auto-guessed from a fixed header-alias table) and validation issues for the
  sampled rows, and never touches SQLite.

## Persistence contract

Migration 009 appends, without changing migrations 001–008:

- `contacts.external_id TEXT NULL`
- `CREATE UNIQUE INDEX contacts_external_id_unique ON contacts(external_id) WHERE external_id IS NOT NULL`
  — a partial index, so the many contacts without an external id are never
  compared against each other.

## Application contract

The versioned local API adds four commands (`schemas/v1/local-api.json`):

- `preview_contact_import(path, mapping?)` — read; returns `ContactImportPreview`
  (`headers`, `rowCount`, effective `mapping`, up to 50 `sampleRows`, and
  `issues`).
- `import_contacts(request: ImportContactsRequest)` — write; `request` is
  `{ actor?, path, mapping }` (actor defaults to the `import` actor). Returns
  `ContactImportSummary`: `{ created, updated, skipped: ContactImportIssue[] }`.
- `export_contacts_csv(path)` — write; returns `CsvExportReport { path, rowCount }`.
- `export_opportunities_csv(path)` — write; returns `CsvExportReport { path, rowCount }`.

`ContactImportMapping` names, per import target (`externalId`, `firstName`,
`lastName`, `displayName`, `role`, `kind`, `preferredContactMethod`,
`addressLine1`, `addressLine2`, `city`, `state`, `postalCode`,
`propertyType`, `notes`, `company`, `email`, `phone`, `tags`), the CSV header
it reads from; unset targets are simply not imported. An unknown header in an
explicit mapping is a caller (`invalid_input`) error, not a per-row problem.
Each imported row is validated through the same contact validation interactive
writes use, so imports cannot drift from the UI's rules.

Contact export columns, in order: `id`, `external_id`, `first_name`,
`last_name`, `display_name`, `role`, `kind`, `preferred_contact_method`,
`address_line1`, `address_line2`, `city`, `state`, `postal_code`,
`property_type`, `notes`, `favorite`, `company`, `email`, `phone`, `tags`,
one column per active contact custom field definition (ordered by sort key),
`created_at`, `updated_at`.

Opportunity export columns, in order: `id`, `name`, `contact_display_name`,
`company`, `stage`, `value`, `currency_code`, `probability_percent`,
`expected_close_date`, `source`, `source_label`, `tags`, one column per active
opportunity custom field definition (ordered by sort key), `created_at`,
`updated_at`. `value` is major units (e.g. dollars, two decimal places)
derived from `value_minor`.

Both exports include only active (non-archived) records, ordered by
name/display name then id, and create missing parent directories for the
destination path.

## Verification

`src-tauri/tests/csv_import_export.rs` covers:

- Preview: mapping auto-guess plus reported row issues without writing;
  honoring an explicit mapping and rejecting unknown columns; sampling at most
  50 rows while still counting all of them.
- Import: creating contacts, companies, and tags with `import`-actor command
  log rows; updating matched contacts by external id while skipping invalid
  rows; atomicity (a forced failure leaves nothing behind); RFC 4180 edge
  cases (quoted commas, embedded newlines, blank rows).
- Export: contact export carries metadata columns and round-trips without
  creating duplicates on re-import; opportunity export writes major-unit
  values, stage names, and metadata columns; exporting to a missing directory
  creates it, and exporting again overwrites the existing file.
