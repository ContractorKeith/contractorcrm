# Hand-off envelope and link commands

Status: implemented v1 — exercised end to end 2026-08-19
Updated: 2026-08-19

How an opportunity leaves ContractorCRM: an exported JSON envelope carries the
context, and link commands record where the resulting quote or job lives. The
CRM never writes into another module's database.

## Envelope schema (schemaVersion 1)

`export_handoff_envelope(opportunityId, destinationPath, overwrite)` writes a
pretty-printed JSON file. It refuses an existing destination unless
`overwrite` is set, and logs the export in the command log.

```json
{
  "schemaVersion": 1,
  "kind": "opportunity_handoff",
  "exportedAt": "2026-08-14T18:00:00.000Z",
  "product": { "name": "ContractorCRM", "version": "0.1.0" },
  "opportunity": {
    "id": "…",
    "name": "Backyard privacy fence",
    "stageId": "stage-won",
    "stageName": "Won",
    "value": { "valueMinor": 250000, "currencyCode": "USD" },
    "probabilityPercent": null,
    "expectedCloseDate": null,
    "source": null,
    "sourceLabel": null,
    "lostReasonId": null,
    "notes": null,
    "quoteRef": {
      "tool": "quoter",
      "externalId": "123",
      "label": "Q-123",
      "linkedAt": "2026-08-14T17:55:00.000Z"
    },
    "jobRef": null,
    "archivedAt": null,
    "createdAt": "…",
    "updatedAt": "…",
    "version": 3
  },
  "contact": { "…contact wire shape including channels…": "or null" },
  "company": { "…company wire shape…": "or null" }
}
```

Field notes:

- Money is always integer minor units plus an ISO currency code — no floats.
- `contact` and `company` are the full wire shapes of the linked records
  (contact includes its channels), or `null` when the opportunity has none.
- `quoteRef` / `jobRef` are the stored hand-off references, or `null`.

The envelope is frozen in `schemas/v1/handoff-envelope.json` — a manifest
pinning every field the exporter writes, including the nullable ones. The
contract test `handoff_envelope_v1_matches_the_frozen_manifest` in
`src-tauri/tests/schema_contracts.rs` exports real envelopes from a seeded
database and compares them field for field: a field the manifest does not list
fails the test, and so does a manifest field the export stops writing. Adding
to the envelope is therefore always a deliberate edit of both.

## Versioning rule

- Changes within a major `schemaVersion` are additive only — consumers must
  ignore unknown fields.
- A breaking change (removing, renaming, or retyping a field) bumps
  `schemaVersion` and ships with a migration note in this document.
- Envelope versions are independent of the MCP API version and the archive
  format version (see docs/LOCAL_API.md).

## Field ownership after hand-off

The envelope is a snapshot, not a live link. Once ContractorProject creates the
job, the two modules own different things and neither writes into the other:

| Field | Owner after the job exists | Notes |
| --- | --- | --- |
| Opportunity name, value, stage, source, notes | ContractorCRM | The sales record stays the CRM's. Renaming it here does not rename the job. |
| Contact and company records | ContractorCRM | The job carries a copy of what the envelope said at export time. |
| `quoteRef`, `jobRef` on the opportunity | ContractorCRM | Bookmarks: tool, external id, label, linked timestamp. |
| Job name, status, schedule, crew, costs | ContractorProject | From the moment the job is created, everything about running the work is the job's. |
| Time zone, calendars, working days | ContractorProject | The envelope never carries them; the importer takes its own `--timezone`. |

Practical rules:

- Nothing syncs back automatically. Editing the opportunity does not touch the
  job, and finishing the job does not move the opportunity.
- Re-exporting and re-importing an envelope creates a second job — it is not an
  update. Fix mistakes in whichever module owns the field.
- If the job's name changes in ContractorProject and you want the CRM label to
  match, re-run `link_job` with the new label. That is a CRM-side edit.
- Breaking envelope changes bump `schemaVersion`; consumers pinned to v1 keep
  working until they opt in.

## Link commands

References are stored on the opportunity as tool + external id + label +
linked timestamp. Every command is version-checked (`expectedVersion`), bumps
the record version, and writes a command-log row; the actor defaults to
`user`.

- `link_quote(opportunityId, quoteRef, expectedVersion)` — records where the
  quote lives (`quoteRef`: `{ tool, externalId, label? }`).
- `unlink_quote(opportunityId, expectedVersion)` — clears the quote reference.
- `link_job(opportunityId, jobRef, expectedVersion)` — records the
  ContractorProject job created from this opportunity.
- `unlink_job(opportunityId, expectedVersion)` — clears the job reference.

## Exercised end to end

The envelope file is the entire interface — neither module links against the
other's code, and neither writes into the other's database. The flow, verified
on 2026-08-19:

1. CRM: create the contact and opportunity, move it into the `won` stage.
2. CRM: `export_handoff_envelope` writes the schemaVersion 1 JSON file.
3. ContractorProject: its `handoff-import` binary
   (`--envelope <path> --database <path> [--timezone <tz>]`) validates
   `schemaVersion` and `kind`, ignores unknown fields, creates a job named
   from the opportunity, and prints one JSON line:
   `{"jobId","jobName","createdAt"}`.
4. CRM: `link_job(opportunityId, { tool: "contractorproject", externalId:
   jobId, label: jobName }, expectedVersion)` records where the job lives.

Run it with `scripts/handoff_e2e.sh` (set `CONTRACTORPROJECT_DIR` when the
sibling checkout is not at `../contractorproject`). The script builds the
sibling binary, exports `HANDOFF_IMPORT_BIN`, and runs
`src-tauri/tests/handoff_e2e.rs`. That test is `#[ignore]`d and skips when the
variable is unset, so CI never needs the sibling repository. Imports are not
deduplicated: importing the same envelope twice creates two jobs.

## Won-stage rule

`link_job` is allowed only while the opportunity sits in a stage of kind
`won`; any other stage returns `validation_failed`
(`opportunity_not_won`) with a clear message. Quotes have no stage
restriction — quoting happens throughout the pipeline.
