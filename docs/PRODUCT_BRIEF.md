# ContractorCRM — Product Brief (v0.1 Planning)

**Working name:** ContractorCRM
**One-liner:** Local-first, AI-native CRM for contractors — contacts, opportunities, and history that live on your machine and connect cleanly to jobs and quotes.

## Vision

ContractorCRM is a module in the OpenContractorOS suite of local-first, AI-native tools for contractors, remodelers, and home-service businesses.

Each module runs fully offline on the user's machine (native Mac and Windows), can be used standalone, and is designed to connect with the others. ContractorCRM focuses on relationships and pipeline so leads and clients are not lost, while handing off cleanly to quotes and to ContractorProject once a job is won.

The overall suite prioritizes data ownership, offline reliability, and practical AI assistance. Open source core (license TBD) with architectural room for optional paid team or cloud features later. Domain undecided.

## Why this is different

- Native desktop and offline-first instead of browser/SaaS CRMs (HubSpot, Salesforce, Jobber, JobNimbus, etc.).
- Built specifically for small contractors and trades rather than generic sales teams.
- Simple ACT!-style contact + opportunity + history model instead of heavy marketing automation or enterprise complexity.
- Designed from the start to link opportunities to quotes and to Projects/Jobs.
- AI-native with a local agent API while keeping all data on-device by default.

## Target Users

Solo contractors, small trades businesses, remodelers, and home-service operators who need to track leads, clients, subcontractors, and follow-ups without paying per-seat SaaS fees or giving up data ownership.

## MVP Goals

Deliver a clean, fast, usable local CRM that handles contacts, a simple pipeline, activity history, and follow-ups, with clear links into the rest of the suite and meaningful AI assistance.

## Core Features (Prioritized)

### Must have (v1)

- Contacts and companies (clients, leads, subs, vendors)
- Simple customizable pipeline / opportunities with stages, value, and expected close
- Activity timeline (calls, notes, site visits, follow-ups) on contacts and opportunities
- Tasks and reminders
- Fast local search and basic saved views/filters
- Link opportunity → quote and (when won) → ContractorProject job
- Local database, offline-first, native Mac + Windows packaging
- Natural-language assistance + basic AI summaries and next-action suggestions
- Documented local API for agents

### Should have

- Tags, custom fields, and simple segmentation
- Document/photo attachments on contacts and opportunities
- Lost-reason tracking
- "Needs attention" views for stale leads or overdue follow-ups
- Basic templates for follow-up notes

### Later / Suite foundations

- Shared contact records across modules
- Deeper activity sync with Projects
- Optional local multi-user mode

## Technical Principles

- Local-first (SQLite or equivalent)
- Offline by design
- AI via BYOK + local models; data stays on the machine by default
- Modular and loosely coupled so it connects to Projects, quotes/markup, and future modules without becoming a monolith
- Open source core; architecture must not block future paid team or hosted options

## Explicitly Out of Scope for v1

Marketing automation, email campaigns, complex lead scoring, full email client, heavy reporting dashboards, forced cloud sync, mobile apps, enterprise permissions and multi-tenant features.

## Success Criteria for Initial Planning

- Clear data model for contacts, companies, opportunities, and activities
- Clean hand-off path from opportunity → quote → Project/Job
- Realistic MVP that feels fast and native
- Local API surface defined early
- Packaging approach consistent with ContractorProject

**License:** Open source (exact license TBD)
**Domain:** Undecided
