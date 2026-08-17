# Tags and typed custom fields implementation brief

Issue: #18
Status: contract frozen; implementation in progress
Updated: 2026-08-17

## Boundary

Add flat tags and user-defined text, number, date, and single-select fields to
contacts, companies, and opportunities. Metadata remains local SQLite domain
data behind the Rust application seam. This milestone does not extend FTS,
implement CSV/archive/attachments, add formulas, or enforce required fields.

## Persistence contract

Migration 008 appends these tables without changing migrations 001–007:

- `tags`: id, label, nullable semantic `color_role` (`neutral`, `accent`, or
  `attention`), nullable `archived_at`, timestamps, and optimistic version.
  Labels are unique case-insensitively, 1–80 characters, and at most 100 tags
  may exist.
- `record_tags`: tag id, entity type, record id, and timestamp. Its composite
  primary key prevents duplicate assignments. Lookup indexes cover record and
  tag directions.
- `custom_field_defs`: id, entity type, label, field type, sort key, nullable
  `archived_at`, timestamps, and optimistic version. Labels are unique
  case-insensitively per entity type; each surface is capped at 50 definitions.
- `custom_field_options`: stable id, definition id, label, and sort key.
  Select options are unique case-insensitively per definition, capped at 50,
  and ordered by sort key then id.
- `custom_field_values`: stable id, definition id, entity type, record id,
  exactly one typed value column, and timestamps. A definition has at most one
  value per record. Indexed typed columns support finite saved-view filters.

Foreign keys protect tags, definitions, options, assignments, and values.
Strict triggers reject missing polymorphic records, definition/entity/type
mismatches, and select options owned by another definition. No orphan can be
inserted by a direct repository write.

Archiving a contact, company, or opportunity retains its assignments and
values. Tag and field definitions also archive rather than hard-delete, so
existing data and saved-view references remain recoverable. Renaming and
reordering definitions/options preserve stable ids and values. Field type is
immutable. An option referenced by a value cannot be removed; the user must
clear or change those values first. Purge is outside v1.

## Application contract

The versioned local API adds:

- `list_tags(includeArchived)`, `create_tag`, `update_tag`, `archive_tag`, and
  `unarchive_tag`.
- `list_custom_field_defs(entityType, includeArchived)`,
  `create_custom_field_def`, `update_custom_field_def`,
  `archive_custom_field_def`, and `unarchive_custom_field_def`.
- `get_record_metadata(entityType, recordId)` and
  `set_record_metadata(request)`.
- `match_saved_view(entityType, definition)` returning matching canonical
  record ids in deterministic record-id order.

`set_record_metadata` is a complete replacement of a record's tag ids and
custom-field values. It accepts the owning record's expected version, validates
the entire request before writing, replaces assignments and values, bumps the
owning record's version/timestamp, writes one bounded non-secret audit summary,
and commits once. Stale versions, validation failures, and audit failures leave
the owner, assignments, values, and command log unchanged. Re-submitting an
identical state is an explicit no-op: no audit row and no version bump.

Text is capped at 4,000 characters. Numbers must be finite and within
±1,000,000,000,000,000. Dates are real calendar dates in `YYYY-MM-DD`. Select
values reference one stable option id belonging to the definition. A record is
capped at 20 tags and 50 field values.

Definition option updates use stable option ids for existing options and omit
removed options. New options omit the id. Updating cannot change the definition
entity type or field type. All definition/tag mutations use expected versions,
immediate transactions, and bounded audit summaries.

## Saved-view v2 contract

Saved-view schema v2 keeps the existing sort contract and replaces the filter
with this strict finite envelope:

```json
{
  "schemaVersion": 2,
  "filter": {
    "includeArchived": false,
    "tagIdsAll": ["tag-id"],
    "customFields": [
      {
        "definitionId": "field-id",
        "fieldType": "text",
        "operator": "contains",
        "value": "residential"
      }
    ]
  },
  "sort": { "field": "name", "direction": "ascending" }
}
```

Tag ids use AND semantics and are capped at 20. Custom-field predicates use
AND semantics and are capped at 10. The only operators are:

- text: `contains`, `equals`;
- number: `equals`, `greaterThanOrEqual`, `lessThanOrEqual`;
- date: `on`, `before`, `after`;
- select: `is` with a stable option id.

There are no caller-defined columns, operators, nesting, or SQL fragments.
Creating/updating a view accepts validated v2 only. Reading known unversioned
v0 and v1 definitions returns an in-memory v2 representation with empty tag and
custom-field filters while retaining the original stored bytes. Malformed and
future definitions are rejected without rewriting. `match_saved_view`
validates referenced tags, definitions, entity types, field types, and options;
stale references produce a stable validation error and never silently broaden
results.

## UI contract

- A reusable metadata surface manages tags and field definitions, renders
  current metadata, and edits assignments/values on existing contact, company,
  and opportunity details/edit forms. New records are saved first, then receive
  metadata through the versioned owner seam.
- Definition management uses labelled modal dialogs, native labelled controls,
  keyboard-complete actions, contained focus, Escape/Cancel restoration,
  destructive archive confirmation, alerts for validation/conflict failures,
  and polite success status.
- Record tag controls expose named remove buttons. Text, number, date, and
  select inputs retain their native semantics and labels. Clearing a control
  removes its value.
- Contacts, Companies, and Pipeline expose active tag and typed custom-field
  filters. Matching is supplied by `match_saved_view`; React never interprets
  SQL or reads SQLite. Applying or changing a filter participates in the
  existing saved-view selected/Modified behavior. Pipeline Board remains a
  summary mode and does not expose list saved-view controls.
- Archived definitions/tags remain visible in management and existing record
  metadata, clearly marked archived, but are excluded from new assignments and
  new filter construction.

## Verification

- Upgrade a populated migration-007 database and prove existing canonical
  records, saved-view JSON, FTS content, and versions survive unchanged; verify
  the v8 pre-migration backup and idempotent reopen.
- Rust integration tests cover every entity and field type, assignments,
  no-ops, conflicts, invalid values, lifecycle rules, rollback on forced audit
  failure, restart persistence, v0/v1/v2 saved views, stale references, and
  deterministic matching.
- Frontend tests cover all three surfaces, definition management, all four
  inputs, filtering/saved views, conflicts, empty states, keyboard operation,
  focus containment/restoration, and accessible names/status.
- Packaged macOS acceptance uses an isolated `HOME`, creates and edits metadata
  on all three entities, saves filtered views, restarts, and verifies identical
  definitions, assignments, values, filters, and ordering. AX and keyboard are
  required. Actual VoiceOver speech and Windows/NVDA remain nonblocking release
  hardening gates and must not be inferred from AX output.
