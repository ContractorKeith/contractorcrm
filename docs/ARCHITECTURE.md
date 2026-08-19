# Architecture and stack

Status: implemented through Slice 5 — AI provider adapters and the MCP stdio
helper ship; packaging is the remaining planning baseline
Updated: 2026-08-19

## Recommendation

Build a Tauri 2 desktop app with a React and TypeScript frontend, a Rust application core, and SQLite persistence owned entirely by Rust — the same architecture ContractorProject has already proven. Foundations are copied, never re-decided: ContractorCRM inherits the sibling's stack, data rules, AI rules, and packaging pipeline, and only its domain modules and UI surfaces differ.

## Stack

| Area | Choice | Reason |
| --- | --- | --- |
| Desktop shell | Tauri 2 | Cross-platform native packaging with a small runtime and a Rust host process. Matches ContractorProject. |
| UI | React, TypeScript, Vite | Fast iteration for record lists, detail forms, timeline, and board views. |
| Styling | CSS variables per `docs/design/DESIGN.md` | The Industry token sheet shared with ContractorProject; no external design system. |
| Application core | Rust | One deterministic implementation for validation, persistence, search, backups, agents, and desktop commands. |
| Database | SQLite through `rusqlite` | Embedded, transactional, portable, directly controlled by the core. |
| Local search | SQLite FTS5 | Fast full-text search across contacts, companies, notes, and opportunities without extra processes. |
| IDs and money | UUIDv7-style opaque IDs; integer minor currency units | Portable suite identity and exact arithmetic for opportunity values. |
| Agent interface | MCP over stdio (implemented: `contractorcrm-mcp`) | No listener, port, or cloud dependency; tools call the same application services as the UI. |
| AI integration | Provider adapters plus a local OpenAI-compatible adapter (implemented) | BYOK and local models share one narrow interface; credentials stay in the OS keychain, outside CRM data. |
| Testing | Rust unit/integration tests, Vitest, React Testing Library, Playwright smoke tests | Verify rules at the core; UI tests only for observable workflows. |
| CI/releases | GitHub Actions on macOS and Windows | Builds and tests on the OS that produces each signed package. |

Do not give the frontend direct SQLite access. Tauri commands and MCP tools translate inputs into the same Rust application requests, so validation, transactions, audit data, and error behavior stay in one deep module.

## Shape

```text
React UI ───── Tauri commands ─┐
                              ├─ Application interface ─ Domain modules
MCP helper ─── tool adapter ───┘             │
                                              ├─ SQLite repository (+ FTS5 index)
                                              ├─ backup/export adapter
                                              ├─ suite hand-off adapter
                                              └─ AI provider adapters
```

### Domain modules

- `contacts`: companies and individual contacts, roles, tags, custom fields, favorites
- `pipeline`: opportunities, customizable stages, value/probability/expected close, sources, lost reasons
- `activities`: the timeline — calls, emails, texts, site visits, meetings, notes — on contacts and opportunities
- `tasks`: personal and record-linked tasks, due dates, priorities, reminders, needs-attention rules
- `search`: full-text index maintenance and saved views/filters
- `handoff`: versioned export/link envelopes toward quotes and ContractorProject jobs
- `application`: use cases and transactions exposed to UI and agent adapters
- `ai` (`src/ai.rs`): provider settings, the keychain credential seam, the
  OpenAI-compatible adapter, and the bounded-context preview type
- `proposals` (`src/proposals.rs`): typed drafts, the in-memory draft store and
  its 15-minute TTL, apply, and undo
- `explain` (`src/explain.rs`): plain-language explanations of deterministic
  attention flags
- `followups` (`src/followups.rs`): history summaries, follow-up templates, and
  drafted follow-up tasks
- `mcp` (`src/mcp.rs`): the stdio agent adapter and its tool table; the
  `contractorcrm-mcp` binary only parses the command line

The needs-attention rules ("no contact in 21 days", "proposal sent, no response") are a pure calculation seam: given records and a reference date, they return deterministic flags. They must not know about React, SQLite, Tauri, or AI — the model explains and prioritizes those facts, it never invents them.

### UI slices

- Contact and company list with saved views, recents, and favorites
- Contact/company detail with the activity timeline
- Pipeline view (list plus board — final form per the open DESIGN.md question)
- Opportunity detail with linked quote/job references
- Tasks and needs-attention views
- Assistant affordances where the work happens rather than one panel: a
  proposal dialog with the typed diff, its context preview, and undo; a
  follow-up draft on contacts and opportunities; explanations on the
  needs-attention view; and the provider, key, and agent-access settings in
  Settings

Keep state local to each slice. A small command/query client and focused React state are enough while the Rust core is in-process.

## Local-first data rules

Identical to ContractorProject:

- One application database per installation in the OS application-data directory.
- Consistent online backups (SQLite backup API or `VACUUM INTO`) while the app is open, plus a portable versioned archive for transfer.
- All schema changes are forward migrations with a pre-migration backup.
- Audit timestamps in UTC; user-facing dates rendered in the machine's locale.
- Attachments are referenced files in a managed assets directory, not database blobs.
- Core workflows never require network access.

Crash semantics, the forward-only migration model, the automatic `.bak` files,
and what to do with a damaged database are documented in
[docs/RECOVERY.md](RECOVERY.md).

## AI rules

Identical to ContractorProject:

- AI reads a bounded application projection, not the database file.
- Model output is a typed proposal or explanation — never executable SQL or an implicit write.
- Deterministic validators re-check every proposal before it can be applied; applying is a normal command with a visible diff and one undoable transaction.
- Attention flags start with deterministic rules (stale lead, overdue follow-up, proposal without response); the model explains and drafts, never asserts new facts.
- Provider calls are explicit, showing which provider receives which records. Contact data is more sensitive than schedule data — the context preview must list the specific contacts included.
- API keys live in OS credential storage. Local model endpoints require no cloud key.

## Suite seam

Suite readiness for v1 means stable opaque IDs, explicit entity types and timestamps, versioned export envelopes, idempotent commands where practical, and documented field ownership. It does not mean a shared database, shared runtime, event bus, account system, or cloud sync.

Concrete v1 hand-offs:

- **Opportunity → quote:** the opportunity stores an external quote reference (tool, ID, label). Creating the quote is the other tool's job; the CRM emits a versioned hand-off envelope with contact and opportunity context.
- **Won opportunity → ContractorProject job:** winning an opportunity offers a job hand-off envelope (contact, company, value, notes) that ContractorProject imports through its own versioned interface; the CRM stores the resulting job reference.
- **Shared contacts:** other modules reference CRM contacts by opaque ID through the local agent API; there is no shared contact database in v1.

## Packaging and distribution

Same release matrix and proof chain as ContractorProject: macOS Apple Silicon and Windows x64 first; source build and tests, exact release commit, platform packaging, signing/notarization, installed-app acceptance, publication, and independent public-download verification recorded separately. No auto-update until both platforms have a repeatable signed pipeline.

## Primary references

- ContractorProject `docs/ARCHITECTURE.md` (sibling baseline)
- [Tauri 2 overview](https://v2.tauri.app/start/)
- [SQLite FTS5](https://www.sqlite.org/fts5.html)
- [SQLite online backup API](https://www.sqlite.org/backup.html)
- [MCP transports](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports)
