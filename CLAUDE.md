# ContractorCRM

## Runtime routing

- Claude Code main sessions read `../dotfiles/claude/ORCHESTRATION.md`.
- Other runtimes must not apply Claude's model assignments.

ContractorCRM is a local-first, AI-native CRM for contractors — contacts, opportunities,
and history that live on the user's machine and connect cleanly to jobs and quotes. It is a
module in the OpenContractorOS suite alongside ContractorProject (sibling repo at
`../contractorproject`). Done for v1 means a fast, native Mac + Windows desktop CRM with
contacts, a simple pipeline, activity history, tasks/reminders, local search, a documented
local agent API, and a clean opportunity → quote → ContractorProject job hand-off.

## Status

Core ready — v0.1.0 development. MVP slices 0–3 are implemented and verified:
native contacts/companies, durable SQLite storage and backup/restore, pipeline and
won-opportunity hand-off, activities, tasks, and needs-attention workflows. The
Slice 4 foundation is implemented through migration 006: tested versioned data/API
contracts and a transactionally maintained FTS5 search seam across contacts,
companies, opportunities, and activities. Keyboard-first global search is implemented
with persistent recents, favorite contacts, parent navigation for activity hits, and
accessible macOS dialog behavior. Slice-3 UX cleanup is complete; saved views,
tags/custom fields, data portability, and attachments remain planned in issues #17–#21.

## Planning baseline

- `docs/PRODUCT_BRIEF.md` is the product scope. Start here.
- `docs/FEATURES.md` is the detailed feature list, suite connections, and AI-native touches.
- `docs/ARCHITECTURE.md`, `docs/DATA_MODEL.md`, `docs/LOCAL_API.md`, `docs/MVP_PLAN.md`
  follow the ContractorProject doc layout and inherit its proven decisions.
- `docs/design/DESIGN.md` is the design system (shared Industry foundation with
  ContractorProject) and logo spec.

## Conventions & Gotchas

- Stack and packaging must stay consistent with ContractorProject: Tauri + React +
  TypeScript UI, Rust core, SQLite storage.
- Local-first and offline by design — no network dependency for core CRM work; AI is
  BYOK/local models and never called without explicit user action.
- Route all writes through the Rust application seam; UI and agents never touch SQLite
  directly (same rule as ContractorProject).
- Use contractor-facing language (leads, clients, subs, vendors, jobs), not generic
  sales-team CRM jargon.
- Suite integrations stay behind versioned interfaces — modular hand-offs, no shared
  platform or event bus for v1.

## Out of Scope (v1)

- Marketing automation, email campaigns, lead scoring, full email client, heavy reporting
  dashboards, forced cloud sync, mobile apps, enterprise permissions, multi-tenant.
