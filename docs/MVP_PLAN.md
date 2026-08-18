# MVP delivery plan

Status: active — implemented through Slice 4 CSV contact import/export
Updated: 2026-08-18

The work is sequenced as tracer slices, mirroring ContractorProject's plan. Each slice leaves a usable path through the real desktop app and keeps records, persistence, UI, and agent interfaces aligned. Because the sibling module already proved the stack, ContractorCRM skips the platform spikes and reuses its CI, packaging, and release patterns.

## Release boundary

v1 is complete when a user can manage contacts and companies, run opportunities through a customizable pipeline, log activity history, work tasks and follow-ups, search everything locally, back up and restore, hand a won opportunity off toward a quote and a ContractorProject job, and use AI assistance and a documented local agent interface — in signed macOS and Windows packages.

Marketing automation, email campaigns, lead scoring, a full email client, heavy dashboards, cloud sync, mobile, and multi-user do not block v1.

## 0. Prove the foundation

- [x] Scaffold the Tauri 2 + React + Rust workspace from the ContractorProject structure; linting, tests, macOS and Windows CI green on hello-world.
- [x] Copy the Industry design tokens and base component layer; render one themed window in light and dark.
- [x] Turn the data model and local API drafts into versioned schemas (v1
  manifests in `schemas/`, with table fields/constraints and command
  inputs/outputs verified against live migrations, registry, and wire shapes).
- [x] Decide the pipeline view question from DESIGN.md: table first, read-only board second (decided 2026-08-14; see DESIGN.md open items).

Exit: the team can build, test, and package the shell on both platforms, and the record/pipeline UI approach is decided.

## 1. First durable contact

- [x] SQLite initialization, forward migrations, and application-data path handling (ported from the sibling).
- [x] Create/edit/archive companies and contacts with record-version conflicts, contact channels, and roles.
- [x] Contact and company list plus detail views through the real application interface.
- [x] Consistent backup command and restore verification test.

Exit: a packaged development app can create contacts and companies, restart without data loss, archive recoverably, and restore a verified backup.

## 2. Pipeline

- [x] User-editable stages with won/lost kinds and defaults (Lead → Estimating → Proposal Sent → Negotiation → Won / Lost).
- [x] Opportunities with value (integer minor units), probability, expected close, and source.
- [x] Stage moves with append-only stage history; lost moves require a lost reason.
- [x] Pipeline table view with stage as a column — the primary view; sortable and keyboard-first.
- [x] Read-only kanban board as a summary view (click a card to open the deal); drag-to-move stays out of v1.
- [x] Quote and job reference fields with the versioned hand-off envelope (export side only; see docs/HANDOFF.md).

Exit: a user can run an opportunity from lead to won or lost, see the pipeline at a glance, and the history explains every stage change.

## 3. Timeline, tasks, and attention

- [x] Log calls, emails, texts, site visits, meetings, and notes on contacts, companies, and opportunities.
- [x] Unified timeline rendering with opportunity activities projected onto contact timelines.
- [x] Tasks with due dates, priorities, reminders, and optional log-on-complete.
- [x] Deterministic needs-attention rules (stale lead, overdue follow-up, proposal without response) with configurable thresholds.
- [x] "Last contacted" and next-action columns in list views.

Exit: nothing falls through the cracks — every stale lead and overdue follow-up is visible without opening records one by one.

## 4. Search, views, and data in/out

- [x] FTS5 index across contacts, companies, opportunities, and activities, maintained transactionally.
- [x] Global search with keyboard-first navigation; recents and favorites.
- [x] Saved views with versioned filter and sort definitions.
- [x] Tags and custom fields (text, number, date, select), with versioned saved-view filters.
- [x] CSV contact import with mapping preview and stable external IDs; CSV export for contacts and opportunities.
- [ ] Versioned portable archive export/import with path and checksum validation.
- [ ] Attachments on contacts and opportunities as managed files.

Exit: a contractor can bring in an existing contact spreadsheet, find anything in under a second, and take all their data back out.

## 5. Local AI and agent interface

- [ ] Port the provider interface, OS credential storage, and local OpenAI-compatible adapter from the sibling.
- [ ] Natural-language record creation and updates as typed proposals with diffs, validation, explicit apply, undo, and audit records.
- [ ] History summaries, next-action suggestions, and follow-up drafting from templates.
- [ ] AI explanations layered on the deterministic attention flags.
- [ ] MCP stdio helper with read-only/read-write onboarding and the contact-data context preview.
- [ ] Document and test every MCP tool, size limit, error kind, and version conflict.

Exit: the app remains fully useful with AI disabled; with AI enabled, no model output mutates data without validated user acceptance, and no contact data reaches a provider without an explicit, inspectable call.

## 6. Suite hand-off and hardening

- [ ] Exercise the won-opportunity → ContractorProject job hand-off end to end against the sibling's local API; record the job reference back.
- [ ] Freeze v1 hand-off envelope schemas and document field ownership.
- [ ] Crash recovery, migration rollback, and corrupted-database guidance.
- [ ] Keyboard navigation, focus behavior, contrast, and screen-reader labels across list, detail, board, and timeline.
- [ ] Test large databases (10k+ contacts) and define supported record/attachment limits.
- [ ] Threat modeling for attachments, imports, local model endpoints, MCP, and provider context.

Exit: the CRM and ContractorProject demonstrably connect on a real machine without shared infrastructure, and users can safely back up, transfer, and recover their data.

## 7. Release

- [ ] Freeze version and release notes at one exact commit.
- [ ] Pass full macOS and Windows build/test/package matrices.
- [ ] Sign and notarize the macOS package; sign the Windows installer.
- [ ] Run installed-app acceptance on clean user accounts on both platforms.
- [ ] Publish artifacts with checksums and license notices.
- [ ] Independently download, verify, install, launch, and run the core contact-and-pipeline workflow from public artifacts.

Exit: source checks, commit, package, signing, installed acceptance, publication, and public-download verification are independently recorded as passing.

## First implementation issue

Build the thinnest end-to-end slice: create a Tauri window, create one contact through a Rust application command, persist it in SQLite, list it in React, restart, and prove it remains. Do not start the pipeline or timeline until that vertical path is green on macOS and Windows CI.
