# ContractorCRM — Feature List

Beyond local-first + AI-native.

**Positioning:** A simple, contractor-focused CRM in the spirit of old ACT! — contacts, opportunities, and history that live on your machine and connect cleanly to jobs, estimates/quotes, and markup. Built for small contractors, remodelers, and home-service operators who need relationship and pipeline visibility without Salesforce or Jobber-style complexity.

## Core Features

### Contacts & Companies

- Clients, leads, prospects, subcontractors, vendors, and suppliers in one place
- Company + individual contact records with roles (owner, estimator, site contact, etc.)
- Custom fields relevant to trades (service area, property type, preferred contact method, license notes, etc.)
- Tags and simple segmentation
- Full activity history on every contact

### Opportunities / Pipeline

- Simple, customizable pipeline stages (Lead → Estimating → Proposal Sent → Negotiation → Won / Lost)
- Opportunity value, probability, expected close date, and source
- Link an opportunity directly to a quote/estimate and (once won) to a Project/Job
- Lost-reason tracking
- Basic pipeline views (list + kanban-style board)

### Activity & History

- Log calls, emails, site visits, texts, meetings, and notes
- Automatic or manual timeline on every contact and opportunity
- Follow-up tasks and reminders
- "Last contacted" and next-action visibility
- Simple email/phone click-to-log (even if full email sync comes later)

### Tasks & Follow-ups

- Personal and opportunity-related tasks
- Due dates, priorities, and reminders
- "Needs attention" views for stale leads or overdue follow-ups

### Search & Views

- Fast local search across contacts, companies, notes, and opportunities
- Saved filters and lists (e.g., "Hot leads this month", "Clients in [service area]", "Subs for fencing")
- Recent and favorite records

### Documents & Attachments

- Attach plans, proposals, contracts, photos, and notes to contacts or opportunities
- Easy hand-off to the plan-markup tool or Projects module

### Light Communication Support

- Store email addresses and phone numbers cleanly
- Basic templates for common follow-ups or proposal cover notes (AI can help generate these)
- Optional later: local email logging or simple send integration

## Suite Connections (important)

- Opportunity → can create or link to a Quote / Estimate
- Won opportunity → creates or updates a Job in ContractorProject with contact and budget context
- Contacts and companies shared (or easily referenced) across Projects, Quotes, and Markup
- Activity on a job can roll back to the contact timeline
- Consistent local data conventions so modules stay loosely coupled but useful together

## Explicitly Keep Light (for now)

- No heavy marketing automation
- No complex lead scoring engines
- No forced multi-user enterprise permissions in v1
- No mandatory cloud sync
- Avoid turning it into a full Jobber/JobNimbus clone

## AI-Native Touches (on top of the local foundation)

- Natural language contact/opportunity creation and updates
- Suggested next actions or follow-up wording
- Summaries of a contact's history or an opportunity's status
- Risk flags ("no contact in 21 days", "proposal sent but no response")
- Local agent API so other tools or agents can read/write CRM data

---

This keeps the CRM practical for the same audience as the rest of the suite: people who want ownership of their client and pipeline data, offline reliability, and just enough structure to stop losing leads or forgetting follow-ups — without the overhead of modern SaaS CRMs.
