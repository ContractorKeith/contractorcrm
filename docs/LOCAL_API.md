# Local agent API

Status: v1 application command contract implemented; MCP adapter planned
Updated: 2026-08-16

The implemented command registry, named inputs, outputs, foundational wire
types, and stable error kinds are published in `schemas/v1/local-api.json` and
verified by `src-tauri/tests/schema_contracts.rs`. Tools described below that
do not yet appear in that manifest remain planned work.

## Interface

Ship an MCP helper with the desktop application and use stdio as the v1 transport, exactly like ContractorProject. The agent client launches the helper; ContractorCRM does not open a network listener for normal single-user use.

The MCP adapter calls the same Rust application interface as the desktop UI. It never opens SQLite directly and cannot bypass validation, record-version checks, or audit logging.

## Initial tools

### Read

- `search_records(query, entityTypes?, limit?)` — bounded FTS-backed search across contacts, companies, opportunities, and activities; activity hits include their parent navigation target
- `list_contacts(kind?, tag?, limit?, cursor?)`
- `get_contact(contactId, include?)` — include options: `activities`, `tasks`, `opportunities`, `customFields`
- `list_companies(kind?, limit?, cursor?)`
- `get_company(companyId, include?)`
- `list_opportunities(stageId?, status?, limit?, cursor?)`
- `get_opportunity(opportunityId, include?)`
- `get_timeline(parentType, parentId, window?, limit?, cursor?)`
- `list_tasks(status?, dueBefore?, parentType?, parentId?, limit?, cursor?)`
- `get_attention_flags(asOfDate?)` — deterministic stale-lead / overdue / no-response flags

### Propose

- `propose_record(kind, description)` — natural-language contact/company/opportunity creation as a typed draft
- `propose_update(entityType, entityId, request, expectedVersion)`
- `propose_followup(parentType, parentId, objective?)` — drafts follow-up wording plus an optional task
- `summarize_history(parentType, parentId, window?)` — explanation only, no proposal ID
- `explain_attention_flag(flagId)` — explanation only

Proposal tools return a typed diff, warnings, affected versions, and an opaque proposal ID. They do not mutate CRM data.

### Write

- `apply_proposal(proposalId, expectedVersions)`
- `create_contact(contact)` / `create_company(company)`
- `update_contact(contactId, patch, expectedVersion)` / `update_company(companyId, patch, expectedVersion)`
- `create_opportunity(opportunity)`
- `update_opportunity(opportunityId, patch, expectedVersion)`
- `move_opportunity_stage(opportunityId, stageId, lostReasonId?, expectedVersion)`
- `log_activity(parentType, parentId, activity)`
- `create_task(task)` / `complete_task(taskId, expectedVersion, logActivity?)`
- `link_quote(opportunityId, quoteRef, expectedVersion)`
- `link_job(opportunityId, jobRef, expectedVersion)` — records the ContractorProject hand-off result

Write tools are available only in read-write mode. Agent onboarding makes the selected mode visible and reversible.

## Error contract

Stable machine-readable error kinds, shared with the sibling module where meanings overlap:

- `not_found`
- `invalid_input`
- `validation_failed`
- `version_conflict`
- `read_only`
- `proposal_expired`
- `missing_lost_reason` — moving to the lost stage without a reason
- `provider_unavailable`

Validation failures include field paths and safe remediation details. Version conflicts return the current version and require an intentional refresh; they never silently overwrite newer work.

## Context and privacy

CRM data is personal data — names, phones, emails, addresses. The privacy rules are stricter than the sibling's:

- Read tools return bounded projections selected by record and requested fields; timeline bodies are truncated unless explicitly requested.
- Agent responses omit attachment bodies and provider credentials.
- Local MCP reads do not imply permission to send contact data to a model provider; provider calls are separate and show exactly which contacts are included.
- Each mutation records actor, client name, command ID, timestamp, and a concise non-secret summary in the command log.
- Implemented desktop commands use explicit size limits. The future MCP adapter
  will add cursors to list/timeline tools where result sets can grow; the v1
  desktop search command deliberately returns one bounded page (maximum 50).

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
