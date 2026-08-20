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
won-opportunity hand-off, activities, tasks, and needs-attention workflows. Slice 4
(search, views, and data in/out) is complete through migration 010: tested versioned
data/API contracts and a transactionally maintained FTS5 search seam across contacts,
companies, opportunities, and activities. Keyboard-first global search is implemented
with persistent recents, favorite contacts, parent navigation for activity hits, and
accessible macOS dialog behavior. Slice-3 UX cleanup is complete; saved views
with versioned filter/sort definitions are implemented for Contacts, Companies,
and Pipeline. Flat tags and typed text, number, date, and single-select custom
fields are implemented across those same record surfaces, including saved-view
schema v2 metadata filters. CSV contact import with mapping preview and stable
external ids, and CSV export for contacts and opportunities, are implemented.
Versioned portable archive export/import (checksum- and referential-integrity-
verified ZIP, full-replace with a pre-import safety backup) is implemented.
Attachments as managed files on contacts and opportunities (sanitized on-disk
copies under the app data directory, exported/imported as real files inside the
portable archive) are implemented, closing issues #19–#21. Slice 5 (local AI
and the agent interface) is complete, closing issues #31–#37: a narrow
provider seam (local OpenAI-compatible endpoints and BYOK through one
interface, API keys in the OS keychain, provider network calls never under
the storage mutex); natural-language record creation/updates as typed
proposals (field-level diffs, deterministic re-validation, explicit apply,
version-pinned undo, audited, held in memory with a 15-minute TTL — only
apply_proposal writes); history summaries, next-action suggestions, and
follow-up drafting from built-in templates that work verbatim with AI off;
AI explanations layered on the untouched deterministic attention flags; and
a contractorcrm-mcp stdio helper (39 tools, read-only by default, write
tools only with --read-write, preview_context shows exactly what a provider
call would send before it goes out — on both the MCP and desktop surfaces).
docs/SLICE5_COVERAGE.md maps every tool, limit, error kind, and version-
conflict path to its documentation and tests. The slice passed an
independent full-diff review; all findings were fixed before acceptance.
Slice 6 (suite hand-off and hardening) is complete, closing issues #29 and
#38–#44: the won-opportunity → ContractorProject hand-off is exercised end
to end on a real machine (scripts/handoff_e2e.sh drives export → the
sibling's handoff-import binary → link_job; the versioned envelope file is
the only interface); the v1 envelope schema is frozen
(schemas/v1/handoff-envelope.json + a negatively-tested contract test) with
field ownership documented in docs/HANDOFF.md; crash recovery is proven
(SIGKILL-during-write and failing-migration tests, zero-byte and damaged
files refused with guidance, docs/RECOVERY.md); the accessibility pass
covers keyboard, focus, labels, and contrast (one measured token fix)
across all surfaces; a 10k-contact database is usable (migration 011 +
one-query channel batching took list_contacts from 51s to ~0.4s; RecordTable
virtualizes above 150 rows with the keyboard model preserved; limits in
docs/DATA_MODEL.md); and docs/THREAT_MODEL.md models attachments, imports,
model endpoints, MCP, and provider context, with three fixes landed
(Windows drive-relative path escape, CSV import bounds, MCP message bounds)
and follow-ups filed as #46–#49. The slice passed an independent full-diff
review across both repos and on-demand native verification on macOS and
Windows. Slice 7 (release) is under way: the release boundary is verified in a
real packaged macOS build (`.app` + `.dmg`) and recorded in
docs/release/ACCEPTANCE.md, which fixed three packaged-only breaks — the bundle
shipped the MCP helper as the app executable, opening an attachment was refused
by the opener capability, and the agent command line named a binary that is not
on `PATH`. The MCP helper ships inside the app bundle next to the app
executable. v0.1.0 was published 2026-08-20 (AGPL-3.0): a tag-triggered
release workflow builds both platforms with skippable signing, the macOS app
AND dmg wrapper are signed + notarized (Gatekeeper-verified), Windows ships
unsigned by explicit decision with the SmartScreen caveat documented, and the
release carries SHA-256SUMS + THIRD_PARTY_NOTICES. Installed acceptance
passed on macOS from the real artifact; the independent public-download
verification passed unauthenticated end to end; both are recorded in
docs/release/ACCEPTANCE.md. The repo is public. Still open: Keith's Windows
installed-acceptance run (checklist in ACCEPTANCE.md) and the post-v0.1.0
backlog (#27, #46–#49, #56–#57, #59–#64).

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

## Documentation

Canonical user docs for ContractorCRM live in the website repo:
**`ContractorKeith/opencontractoros` → `src/content/docs/crm/`** (served at
opencontractoros.com/docs/crm/).

**Hard rule:** any PR or commit that changes user-facing behavior must include
a matching docs update at that exact path, committed and pushed in the same
working session. Every PR must carry a `docs-updated` or `docs-n/a` marker in
its body or labels — `.github/workflows/docs-reminder.yml` fails it otherwise.

<!-- kodade:kodmem-project:v1:start -->
Follow the managed KödMem project-context rule in `AGENTS.md`.
<!-- kodade:kodmem-project:v1:end -->
