# ContractorCRM v0.1.0

ContractorCRM is a desktop CRM for contractors — the contacts, companies, and
opportunities you actually work, with the history that explains them. It runs on
your Mac or Windows machine and keeps everything in a single local SQLite
database in your app data folder: your client list, your pipeline, and your
notes stay on your machine, and nothing leaves it unless you export a file or
explicitly ask the optional AI assistant to help.

This is the first release. It covers the daily work — contacts, pipeline,
follow-ups, search, and getting your data in and out — and it is honest about
what it does not cover yet.

## What it does

### Contacts and companies

- Clients, leads, subs, vendors, and suppliers in one list, with company records
  and the people attached to them.
- Roles, contact channels (phone, email, and the rest) with a preferred method,
  addresses, property type, service area, and license notes.
- Archive a record instead of deleting it; archived records stay recoverable.
- Every edit is version-checked, so two windows or an agent cannot silently
  overwrite each other.

### Pipeline

- Editable stages with won and lost kinds — Lead, Estimating, Proposal Sent,
  Negotiation, Won, Lost out of the box.
- Opportunity value, probability, expected close date, and source.
- Every stage move is recorded permanently, and a move to Lost requires a lost
  reason, so the history explains itself later.
- A sortable, keyboard-first pipeline table as the main view, plus a read-only
  board for a glance at where everything sits (drag-to-move is not in this
  release).

### History, tasks, and what needs attention

- Log calls, emails, texts, site visits, meetings, and notes on contacts,
  companies, and opportunities.
- One timeline per record, with opportunity activity showing up on the contact's
  timeline too.
- Tasks with due dates, priorities, reminders, and an option to log an activity
  when you complete one.
- Needs-attention rules you can tune — stale lead, overdue follow-up, proposal
  sent with no response — with "last contacted" and next-action columns in the
  lists.

### Search, views, tags, and custom fields

- Full-text search across contacts, companies, opportunities, and activities,
  kept current with every write. Opens from the keyboard, returns in
  milliseconds, and remembers recents and favorites.
- Saved views with your own filters and sort order on Contacts, Companies, and
  Pipeline.
- Flat tags, plus your own text, number, date, and single-select fields on those
  same records — and you can filter saved views by them.

### Your data in and out

- CSV contact import with a column-mapping preview before anything is written,
  and stable external IDs so re-running an import updates rows instead of
  duplicating them.
- CSV export for contacts and opportunities.
- A portable archive — one ZIP with all your records and your attached files —
  that exports, verifies, and imports back. Import replaces the whole database
  and takes a safety backup of the current one first. Checksums and referential
  integrity are checked before a single row lands.
- Attachments on contacts and opportunities: the app keeps its own copy of the
  file in its data folder, so moving or deleting the original does not break the
  record, and the files travel inside the portable archive.
- Backup and restore of the database on demand, with restore verification and
  documented recovery steps for a crash, an interrupted upgrade, or a damaged
  file (`docs/RECOVERY.md`).

### Hand-off to ContractorProject

- A won opportunity exports a versioned hand-off envelope — the opportunity, its
  contact, and its company — that ContractorProject imports as a job. The job
  reference comes back onto the opportunity.
- The envelope file is the entire interface. No shared database, no background
  service, no requirement that both apps be running.

### AI assistant (off by default)

- Everything above works with the assistant switched off. It is off until you
  turn it on and point it at a local model or your own API key.
- With it on: describe a contact, company, or opportunity change in plain
  language and get a typed draft with a field-by-field diff. Nothing is written
  until you accept it, it runs through the same validation as a hand edit, and
  you can undo it.
- History summaries, suggested next actions, and follow-up drafts from your own
  templates — the templates work verbatim without the assistant.
- The attention flags stay deterministic; the assistant explains them, it does
  not invent them.
- Before any call goes out you can preview exactly what record data it would
  include. API keys go in the OS keychain, not a config file.

### Agent access

- A `contractorcrm-mcp` stdio helper lets Claude, another agent, or your own
  script read and write CRM data through the same validated commands the app
  uses.
- Read-only unless you launch it with `--read-write`. Every tool, size limit,
  and error is documented in `docs/LOCAL_API.md`.

## What it does not do

- No marketing automation, email campaigns, or lead scoring.
- No email client, inbox sync, or sending mail from the app.
- No reporting dashboards or forecasting beyond the pipeline views.
- No cloud sync and no account — moving data between machines is the portable
  archive or your own backup.
- No mobile app. Desktop only: macOS and Windows.
- Single user. No sharing, no permissions, no multi-tenant.
- No quoting or estimating in the CRM itself — an opportunity links to a quote
  and hands off to a ContractorProject job.

## Supported limits

Enforced — the app refuses to cross these:

| Limit | Value |
| --- | --- |
| Attachment file size | 256 MiB per file |
| Portable archive total | ~1 GiB uncompressed (checked on export and import) |
| CSV import file size | 64 MiB |
| CSV import rows | 200,000 data rows |
| Saved views | 50 per surface |
| Search results | 50 per query (25 by default) |
| Agent message size | 8 MiB per request line |
| Agent list results | 500 rows per call; 200 timeline entries per call |
| AI draft lifetime | 15 minutes, held in memory only |

Tested to — measured, not enforced. A seeded database of 10,000 contacts, 2,000
companies, 5,000 opportunities, 30,000 activities, and 5,000 tasks (62 MiB on
disk) on an Apple Silicon laptop: the contact list renders in under half a
second, search returns in under 2 ms, and opening the database is effectively
instant. Long lists window their rows, and the pipeline board draws at most 100
cards per column (the list view is the complete one).

There is no cap on total records, attachments, or database size. Past those
tested numbers you are in untested territory, not blocked. Full detail:
`docs/DATA_MODEL.md`.

## Known issues

The threat model (`docs/THREAT_MODEL.md`) was written against this release. Three
findings were fixed before it shipped; four are open as hardening follow-ups.
None of them affect normal day-to-day use — they matter when you open a file
that came from someone else.

- **Attachments open with whatever extension the file has**
  ([#47](https://github.com/ContractorKeith/contractorcrm/issues/47)). Clicking
  Open on an attachment hands the file to the operating system. If the file came
  from a portable archive somebody else gave you, that file could be a program
  rather than a document, and the app does not currently warn you. Treat
  archives from other people the way you would treat any downloaded file, and
  when in doubt open the attachment from your file browser instead.
- **Importing a very large archive uses a lot of memory**
  ([#46](https://github.com/ContractorKeith/contractorcrm/issues/46)). Archive
  import reads the whole archive into memory, so a near-1 GiB archive can push
  memory use to roughly twice that and, on a small machine, stall or crash the
  import. Your existing database is not touched when that happens.
- **Archive import trusts more than the app's own forms do**
  ([#49](https://github.com/ContractorKeith/contractorcrm/issues/49)). Import
  checks structure, checksums, and references, but not every field rule the app
  enforces when you type. A hand-edited or malicious archive can therefore land
  a record the app later refuses to display. The pre-import safety backup is the
  way back.
- **Read-only agent access is enforced in one layer, not two**
  ([#48](https://github.com/ContractorKeith/contractorcrm/issues/48)). The MCP
  helper's read-only mode blocks every write tool correctly today, but the
  database connection underneath is still opened read-write. No known way to
  write through it; it is being tightened as defense in depth.

<!-- SIGNING-CAVEAT: orchestrator to fill in after the Slice 7 signing checkpoint
     (issue #53). Cover code signing / notarization status per platform and the
     exact macOS Gatekeeper and Windows SmartScreen prompts a user should expect
     on first launch, or state that both packages are signed and no warning
     appears. Do not ship this file with this comment still in it. -->

## Installing

Download links, checksums, and step-by-step install instructions for macOS and
Windows: <https://opencontractoros.com/docs/crm/install>
