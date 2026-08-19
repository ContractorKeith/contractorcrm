# Local agent API

Status: v1 application command contract implemented; MCP adapter implemented
and covered end to end (see `docs/SLICE5_COVERAGE.md`)
Updated: 2026-08-19 (Slice 5 docs and test sweep)

The implemented command registry, named inputs, outputs, foundational wire
types, and stable error kinds are published in `schemas/v1/local-api.json` and
verified by `src-tauri/tests/schema_contracts.rs`. Tools described below that
do not yet appear in that manifest remain planned work.

## Interface

Ship an MCP helper with the desktop application and use stdio as the v1 transport, exactly like ContractorProject. The agent client launches the helper; ContractorCRM does not open a network listener for normal single-user use.

The MCP adapter calls the same Rust application interface as the desktop UI. It never opens SQLite directly and cannot bypass validation, record-version checks, or audit logging.

## Agent onboarding

The helper ships as a second binary, `contractorcrm-mcp`, alongside the
desktop app (`src-tauri/src/mcp.rs` is the server; the binary only parses the
command line). It speaks JSON-RPC 2.0 over stdio — one message per line, MCP
revision `2025-06-18` (older `2025-03-26` and `2024-11-05` clients are accepted
and echoed back). `initialize` reports the product version and the local API
version; closing stdin is the graceful shutdown. No socket is opened.

Wire it into an agent client with the command line Settings → Backup & Data →
"Agent access (MCP)" shows for this device:

```
contractorcrm-mcp --database "<app data>/contractorcrm.sqlite3"
contractorcrm-mcp --database "<app data>/contractorcrm.sqlite3" --read-write
```

- **Read-only is the default.** Write tools are not listed at all; calling one
  anyway returns the `read_only` error kind naming the command. The mode is
  whatever the helper was launched with, so it is reversible: drop
  `--read-write` and restart the client.
- A missing database, a file with no readable `schema_migrations` table (it is
  not a ContractorCRM database), or one written by a newer build than the
  helper knows, is refused on stderr with a nonzero exit rather than migrated.
- A database written by an *older* build is refused too in read-only mode: a
  connection the user granted no write permission to never rewrites their
  schema. The message says to launch the desktop app once (or relaunch the
  helper with `--read-write`, which is an explicit write grant and does migrate
  forward, saying so on stderr).
- Every mutating tool call runs as actor `agent` and writes an extra
  `command_log` row naming the MCP client from the `initialize` handshake.
- `preview_context(tool, arguments)` returns the exact bounded projection text
  and `includedRecordRefs` that `summarize_history`, `propose_followup`,
  `explain_attention_flag`, or `propose_update` would send — without calling a
  provider, and without needing the assistant to be on. Each arm takes that
  tool's own arguments; for `propose_followup`, `objective` and `templateId`
  pick the wording rather than the projection and a `window` has no meaning
  (drafting always looks back the default 90 days), so all three are accepted
  and ignored and the preview always matches the real call. The desktop has the
  same seam as the `preview_context` command (see "Initial tools → Propose").
- Bounds over MCP: search returns one page of at most 50; `get_timeline`
  returns at most 200 entries and truncates activity bodies to 500 characters
  unless `fullBodies: true` is passed; list tools accept an optional `limit`
  (maximum 500). No response ever carries attachment bytes, an attachment's
  internal relative path, or a provider credential.

Omitted from the v1 tool surface on purpose — the desktop owns these
file-path and destructive surfaces: `backup_database` / `restore_database`,
`export_archive` / `preview_archive_import` / `import_archive`,
`preview_contact_import` / `import_contacts` / `export_contacts_csv` /
`export_opportunities_csv`, and `add_attachment` / `remove_attachment`.

## Initial tools

### Read

Argument names below are the shipped MCP ones. Related records are fetched with
a second call rather than an `include` option, and lists are bounded by `limit`
rather than paged with a cursor (see "Context and privacy").

- `search_records(query, entityTypes?, limit?)` — bounded FTS-backed search across contacts, companies, opportunities, and activities; activity hits include their parent navigation target. One page, at most 50 rows
- `list_contacts(includeArchived?, limit?)`
- `get_contact(contactId)`
- `list_companies(includeArchived?, limit?)`
- `get_company(companyId)`
- `list_opportunities(includeArchived?, limit?)`
- `get_opportunity(opportunityId)`
- `get_timeline(parentType, parentId, includeRelated?, limit?, fullBodies?)`
- `list_tasks(status?, overdueOnly?, parentType?, parentId?, limit?)`
- `get_attention_flags(referenceTime?)` — deterministic stale-lead / overdue / no-response flags, evaluated against `referenceTime` (UTC ISO-8601) or now
- `list_saved_views(entityType)` — typed, versioned filter/sort definitions for contacts, companies, or opportunities
- `list_tags(includeArchived?)`, `list_custom_field_defs(entityType, includeArchived?)`, and `get_record_metadata(entityType, recordId)`
- `match_saved_view(entityType, definition)` — desktop command; not an MCP tool in v1
- `list_stages()` — pipeline stages in board order with their kind (open, won, lost); the source of the ids `move_opportunity_stage` takes
- `list_lost_reasons()` — the stored lost reasons; moving to a lost stage requires one of these ids
- `list_attachments(parentType, parentId)` — every managed file on a contact or opportunity, oldest first; each returns `id`, `fileName`, `mediaType`, `sizeBytes`, `sha256`, `createdAt`, `version` (never the internal `relative_path`)
- `attachment_path(attachmentId)` — resolves a managed file's absolute path and whether it still exists on disk (`AttachmentLocation { path, exists }`), for the frontend to hand to the OS opener; `exists: false` after a database restore means the row survived but its bytes did not
- `preview_contact_import(path, mapping?)` — parses a CSV file's headers and sample rows without writing; returns the effective mapping (caller's or auto-guessed from header aliases) and per-row validation issues, but does not touch the database. A trailing empty header column (and its cells) is dropped and tolerated; an interior blank header, a duplicate header, or a non-UTF-8/malformed file fails as `invalid_input` (the encoding case with re-save-as-UTF-8 guidance) rather than a partial read.
- `preview_archive_import(path)` — reads a portable archive ZIP and fully
  verifies it without writing anything: untrusted-input size bounds (256 MiB
  per entry, 1 GiB total uncompressed), entry paths, a foreign-product check,
  checksums, `schemaVersion`, `databaseMigrationVersion`, per-row column
  shape against the live schema, the manifest's `recordCounts` against the
  rows actually parsed, referential integrity and structural minimums
  (at least one pipeline with open/won/lost stages) across all 17 archived
  tables, and finally a dry-run apply into a throwaway in-memory database
  that exercises every `UNIQUE`/`CHECK` constraint and the search rebuild.
  Because that dry run is the last step, an empty `issues` list means
  `import_archive` will succeed — it is not just a best-effort prediction.
  Returns `ArchiveImportPreview`: `schemaVersion`, `product`, `exportedAt`,
  `databaseMigrationVersion`, `recordCounts` (the *parsed* counts, verified
  against the manifest's claim), and `issues: ArchiveIssue[]` (`{code,
  message}`, capped at 100 with a trailing `too_many_issues` summary).
  Referential and structural checks — and the dry run — are skipped once an
  earlier stage already reported an issue, so fixing the reported problems
  and re-previewing the same file can surface new issues that were hidden
  behind them. An unreadable file (missing, corrupt, no `manifest.json`) is a
  caller `invalid_input` error rather than a reported issue, since it cannot
  be attributed to a record.

### Propose

- `propose_record(kind, description)` — natural-language contact/company/
  opportunity creation as a typed draft. The configured provider extracts a
  strict-JSON draft (fences, prose, and trailing chatter are tolerated;
  anything unparseable is `validation_failed` / `draft_unreadable`, never a
  panic or a partial write). Values the app doesn't store, unexpected value
  shapes, and names that match no record become `warnings`; the draft itself is
  then validated with exactly the rules `create_contact`/`create_company`/
  `create_opportunity` run. A drafted opportunity may name its contact/company
  and the id is resolved deterministically here — an ambiguous or unknown name
  resolves to nothing and warns. Returns a `Proposal`.
- `propose_update(entityType, entityId, request, expectedVersion)` — loads the
  record (`not_found`), checks `expectedVersion` before the model is asked
  (`version_conflict`), sends a bounded field projection as context (no
  attachment bodies, no credentials) with the record named in the call's
  disclosure list, and diffs the extracted patch against current values so only
  fields that actually change appear. Record links (company/contact) are never
  re-pointed by a plain-language edit. Returns a `Proposal` whose
  `affectedVersions` names the record and the version the draft was built from.
- `summarize_history(parentType, parentId, window?)` — implemented; explanation
  only, no proposal and no writes. Builds a bounded projection of one record's
  own timeline (`window` is a day count, default 90, capped at 3650): the
  record's label, type, and id, then up to 25 entries newest-first with
  summaries and bodies truncated. Related-record activity, other records,
  attachment bodies, and credentials are never included, and the call's
  disclosure list names exactly the one record. Returns `HistorySummary
  { parentType, parentId, endpointHost, local, model, summary,
  suggestedNextActions, includedRecordRefs }`; a prose-only answer parses as
  all summary with no suggestions rather than failing. A switched-off or
  unconfigured assistant is `provider_unavailable` and reads neither the
  credential store nor the network.
- `propose_followup(parentType, parentId, objective?, templateId?)` —
  implemented. Picks a follow-up template (`templateId` outright — an unknown
  id is `not_found`; otherwise the closest match to `objective`, falling back
  to the first template) and drafts the wording. With the assistant on, the
  template body plus the same bounded history projection go through the
  provider, which personalizes it; with the assistant off, the template body
  comes back verbatim, nothing is sent, and no credential is read. Either way
  the result carries a `Proposal` of kind `create_followup_task` for a task
  linked to the record, validated with exactly the rules `create_task` runs.
  Nothing is written until `apply_proposal`; `undo_proposal` drops the created
  task (never deletes it). Returns `FollowupDraft { parentType, parentId,
  templateId, templateName, draftText, usedProvider, endpointHost, local,
  model, includedRecordRefs, warnings, proposal }`.
- `get_followup_templates()` / `set_followup_templates(request)` — the stored
  template set (`{version, templates: [{id, name, body}]}`) in `app_settings`,
  following the AI-settings pattern: the built-in set (call follow-up, proposal
  follow-up, post-site-visit note) is returned until the user edits it, and
  saving an empty list restores it. Names and bodies are required and trimmed,
  at most 20 templates, names 80 characters, bodies 2000; a blank id is derived
  from the name, and duplicate ids are `invalid_input`.
- `explain_attention_flag(flagId)` — implemented; explanation only, no proposal
  and no writes. Re-evaluates the deterministic flags, finds `flagId` (the
  stable `<rule>:<recordId>` identity `get_attention_flags` returns), and sends
  a bounded projection — rule name, its thresholds, the dates that tripped it,
  and the one flagged record's label, type, and id — through the provider seam.
  Never sends other records, timelines, attachment bodies, or credentials.
  Returns `AttentionExplanation { flagId, endpointHost, local, explanation:
  ProviderCompletion { purpose, model, text, includedRecordRefs } }`, so the UI
  can always show which model answered and where the record details went. A
  flag that is no longer current (already handled, or from a stale screen) is
  `not_found`; a switched-off or unconfigured assistant is
  `provider_unavailable` and reads neither the credential store nor the
  network.
- `preview_context(request)` — implemented as both a desktop command and an
  MCP tool. `request` is `{tool}` plus that tool's own identifying arguments:
  `summarize_history` (`parentType`, `parentId`, `window?`),
  `explain_attention_flag` (`flagId`), `propose_update` (`entityType`,
  `entityId`, `expectedVersion` — still version-checked, so a preview cannot
  quietly describe a stale record), or `propose_followup` (`parentType`,
  `parentId`, and `objective?` / `templateId?`, which are accepted and ignored
  because they pick the wording, not the projection). Returns `ContextPreview
  { purpose, contextText, includedRecordRefs }` — exactly what that feature
  would send. No provider is called, no credential is read, and it works with
  the assistant switched off, which is what lets the desktop offer "see what
  will be sent" before the user commits to a call.

Proposal tools return a typed diff, warnings, affected versions, and an opaque proposal ID. They do not mutate CRM data.

Drafts live in the running app's memory only — never in SQLite — and expire 15
minutes after they are created. An unknown, expired, or already-applied id is
`proposal_expired`; the three are deliberately indistinguishable. Undo tokens
follow the same rules: memory only, single use, the same 15-minute window, and
the version the apply left behind is pinned, so an undo can never quietly revert
work done after it. A desktop draft and an MCP draft share nothing but this
behavior — each running process holds its own drafts, so a draft cannot be
proposed in one client and applied from another.

### Write

- `apply_proposal(request)` — `{actor?, proposalId, expectedVersions?}`. Takes
  the draft (single use), re-checks every asserted version plus the version the
  draft was built against, and only then applies it through the same
  `create_*`/`update_*` application code the manual path uses, so validation
  runs again against current data in one transaction. A `version_conflict` or a
  failed re-validation puts the draft back so it can be refreshed and retried.
  Writes the ordinary record `command_log` row (actor from the request: `user`
  from the UI, `agent` from an MCP client) plus one row recording that a draft
  was applied. Returns `ProposalApplied { entityType, entityId, created,
  version, undoToken, undoExpiresAt }`.
- `undo_proposal(request)` — `{actor?, undoToken, expectedVersions?}`. Reverses
  one applied draft in a single version-checked transaction: a created record is
  archived (never hard-deleted), an updated record is restored from the stored
  before-image. The record must still be exactly where the apply left it: the
  post-apply version is checked unconditionally, so a caller that asserts
  nothing gets `version_conflict` rather than a silent revert over work done
  after the apply; `expectedVersions` is an additional guard on top, never a
  substitute. Single use, same TTL as the draft, and audited like any other
  write. Returns `ProposalUndone { entityType, entityId, action, version }`.
- `create_contact(contact)` / `create_company(company)`
- `update_contact(contactId, patch, expectedVersion)` / `update_company(companyId, patch, expectedVersion)`
- `create_opportunity(opportunity)`
- `update_opportunity(opportunityId, patch, expectedVersion)`
- `move_opportunity_stage(opportunityId, stageId, lostReasonId?, expectedVersion)`
- `log_activity(parentType, parentId, activity)`
- `create_task(task)` / `complete_task(taskId, expectedVersion, logActivity?)`
- `link_quote(opportunityId, quoteRef, expectedVersion)`
- `link_job(opportunityId, jobRef, expectedVersion)` — records the ContractorProject hand-off result
- `create_saved_view(request)` / `update_saved_view(request)` / `delete_saved_view(request)` — version-checked local list configuration; definitions are validated, bounded, and never interpreted as SQL
- `create_tag` / `update_tag` / `archive_tag` / `unarchive_tag`, matching custom-field-definition lifecycle commands, and `set_record_metadata(request)` — typed, optimistic, audited local metadata writes; identical metadata replacement is a no-op
- `add_attachment(parentType, parentId, sourcePath)` — copies a file from `sourcePath` into the managed attachments store and records it against a contact or opportunity that must exist and not be archived. Refuses a `sourcePath` that already resolves inside the managed root (attaching a managed file to itself). The file name is sanitized (path separators, control characters, and invisible Unicode format characters — bidi overrides, zero-width spaces, and similar — stripped so a right-to-left override can't disguise an executable as a PDF; trailing dots/spaces trimmed; Windows reserved device names prefixed with `_`; length capped at 120 bytes with the extension preserved) and becomes both the stored display name and the on-disk file name; `mediaType` is looked up from the file extension. Files over 256 MiB fail `validation_failed` / `file_too_large`. The file is copied and hashed before the row is written; a failed write removes the copy so a managed file never outlives a failed command. Returns `Attachment`.
- `remove_attachment(attachmentId, expectedVersion)` — version-checked; deletes the row, then best-effort deletes the managed file. Returns `AttachmentRemoval { fileRemoved }` — `false` means the row is gone but its bytes could not be cleaned up, which is harmless since nothing lists or exports an attachment with no row.
- `import_contacts(request)` — applies a mapped CSV file in one transaction; rows match an existing contact by `external_id` then record id. Matched updates are patches: columns the file does not map, and mapped cells that are blank, never overwrite a stored value; `kind` and company link only change when the file carries a non-blank cell for them; `favorite` is never touched by import. A mapped first/last-name cell only re-derives `display_name` when the stored `display_name` was itself derived from the stored name parts — a curated (hand-edited) display name is never overwritten by a name column. Channels (email/phone) are additive — a value not already on the record is inserted, never deleted or replaced; a leading `'` that only exists to defuse a formula trigger (see export below) is stripped from a mapped cell before it's read, so a file this app exported round-trips byte-for-byte instead of storing an escaped value. Tags are additive. Rows matching an archived contact are skipped and reported, never mutated. Invalid rows (including interior blank or duplicate CSV headers, which fail the whole file as `invalid_input`) are skipped and reported as `{line, reason}`; a trailing empty header column is tolerated and dropped rather than rejected. Command log rows use the `import` actor.
- `export_contacts_csv(path, overwrite)` / `export_opportunities_csv(path, overwrite)` — write every active contact or opportunity to a CSV file, including tags and custom field columns. An existing file at `path` without `overwrite: true` fails `validation_failed` with code `destination_exists`; a `path` that resolves to the live database file, one of its `-wal`/`-shm` sidecars, or a `<database>.*.bak` safety copy fails `validation_failed` with code `destination_is_database`, so an export can never overwrite the data it's meant to preserve. Cells beginning with `=`, `+`, `-`, `@`, tab, or carriage return are prefixed with exactly one `'` to block spreadsheet formula injection; `import_contacts` strips that same single leading quote back off, so the guard round-trips safely instead of accumulating quotes on repeated export/import cycles. Each export writes one `command_log` row with `entity_type` `"export"`.
- `export_archive(path, overwrite)` — writes a versioned portable archive ZIP
  (`manifest.json`, `data/<table>.json` for all 17 canonical tables,
  `csv/contacts.csv` + `csv/opportunities.csv` convenience copies, and
  `assets/<attachment id>/<file name>` for every managed attachment file) of
  every canonical record, active or archived. Sweeps any `.import-staging-*`
  directory a previous crashed import left in the attachments root before it
  reads the store. Sums the archived table/CSV bytes and each attachment's
  `size_bytes` as it assembles the archive and refuses
  (`validation_failed` / `archive_too_large`) before writing anything once
  the running total would exceed the ~1 GiB total-uncompressed limit import
  reads archives under — an archive too large to ever import back is refused
  at export time. Refuses (`validation_failed` / `attachment_file_missing`)
  if a referenced attachment's managed file is no longer on disk, rather than
  exporting an archive that can never be imported. Same destination guards as
  the CSV exports: an existing file at `path` without `overwrite: true` fails
  `validation_failed` / `destination_exists`; the live database file (or a
  `-wal`/`-shm`/`.bak` sibling) fails `validation_failed` /
  `destination_is_database`. Returns `ArchiveExportReport { path,
  recordCounts, fileCount }`. Writes one `command_log` row (`entity_type:
  "export"`, `entity_id: "archive"`, actor `user`).
- `import_archive(path)` — verifies the archive exactly as
  `preview_archive_import` does (culminating in the dry-run apply into a
  throwaway in-memory database) and fails `validation_failed` /
  `archive_invalid` (naming the first reported issue) if any issue is found.
  Only once verification passes with no issues does it take a timestamped
  safety backup of the live database (`<database>.pre-import-<stamp>.bak`)
  and, in a single transaction, delete every canonical table in reverse
  dependency order, insert every archived row in dependency order, and
  rebuild the FTS index. If that real apply unexpectedly fails despite the
  dry run passing, the transaction rolls back, the orphaned safety backup is
  removed, and the failure is reported as `validation_failed` /
  `archive_invalid` — the live database is left exactly as it was.
  `command_log`, `app_settings`, and `schema_migrations` are untouched by
  import. Sweeps any leftover `.import-staging-*` directory in the
  attachments root first, then stages the verified attachment files into a
  fresh one of its own before the transaction runs (so a filesystem failure
  never touches the live database). Once the transaction commits, the import
  is done as far as the caller and the database are concerned; the swap into
  the live attachments root (clear the old managed files, move the staged
  ones in) is attempted, retried once on failure, and then abandoned silently
  — `import_archive` always returns its `ArchiveImportReport` rather than
  surfacing a post-commit filesystem error the caller can't undo. A swap that
  never completes leaves some attachments reporting `exists: false` from
  `attachment_path` until the same archive is imported again. Returns
  `ArchiveImportReport { recordCounts, safetyBackupPath }`. Writes one
  `command_log` row (`entity_type: "import"`, `entity_id: "archive"`, actor
  `user`) naming the safety backup path. Full replace only in v1 —
  merge-import is out of scope.

Write tools are available only in read-write mode. Agent onboarding makes the selected mode visible and reversible.

## Error contract

Stable machine-readable error kinds, shared with the sibling module where
meanings overlap. The published list is `errorKinds` in
`schemas/v1/local-api.json`, checked against the code by
`src-tauri/tests/schema_contracts.rs`:

- `not_found`
- `invalid_input` — malformed or unusable arguments, including an unusable `limit`
- `validation_failed` — a record rule refused the write; carries `code` and `field`
- `version_conflict` — carries `resource`, `recordId`, `expectedVersion`, `currentVersion`
- `read_only` — a write tool on a read-only connection; carries `command`
- `proposal_expired` — the draft is unknown, expired, or already applied; carries `proposalId`
- `missing_lost_reason` — moving to the lost stage without a reason
- `provider_unavailable` — the assistant is off, unconfigured, or the endpoint could not answer; carries safe `reason` text, never a key
- `invalid_stored_data` — a stored row or setting could not be read as its current shape
- `backup_failed`, `restore_invalid` — the backup and restore commands (desktop only)
- `storage_unavailable` — the local database itself refused the work
- `io` — a filesystem failure behind an attachment, export, or archive command

The MCP adapter returns the same JSON the desktop gets: a tool failure comes
back as `isError: true` with the error under
`structuredContent.error`, kind and details intact. Validation failures include
field paths and safe remediation details. Version conflicts return the current
version and require an intentional refresh; they never silently overwrite newer
work.

## Context and privacy

CRM data is personal data — names, phones, emails, addresses. The privacy rules are stricter than the sibling's:

- Read tools return bounded projections selected by record and requested fields; timeline bodies are truncated unless explicitly requested.
- Agent responses omit attachment bodies and provider credentials.
- Local MCP reads do not imply permission to send contact data to a model provider; provider calls are separate and show exactly which contacts are included.
- Each mutation records actor, client name, command ID, timestamp, and a concise non-secret summary in the command log.
- Every read is bounded by an explicit size limit, not by paging: search returns
  one page of at most 50, `get_timeline` caps at 200 entries with 500-character
  bodies, and the list tools take an optional `limit` of 1–500. A `limit` of 0
  or one over the cap is `invalid_input` rather than a silent clamp, so a client
  never believes it read everything. Cursors stay future work — they only earn
  their complexity if a single contractor's list genuinely outgrows 500 rows.
- Every tool, size limit, error kind, and version-conflict path is mapped to its
  documentation and its test in `docs/SLICE5_COVERAGE.md`.

## Suite hand-off surface

Other OpenContractorOS modules are just MCP clients of this helper. The hand-off flow for a won opportunity:

1. Caller reads the opportunity and contact context.
2. Caller creates the job in ContractorProject through that module's own interface.
3. Caller records the result here via `link_job`.

The CRM never writes into another module's database, and vice versa. Envelope schemas are versioned independently of the MCP API version.

The envelope schema, link commands, and won-stage rule are documented in `docs/HANDOFF.md`.

## Versioning

- The helper reports product and API versions during initialization.
- Tool input schemas are additive within a major version.
- Breaking changes require a new major API version and a migration guide.
- Archive versions, hand-off envelope versions, and MCP API versions are independent.
