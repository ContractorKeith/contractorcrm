# Hand-off envelope and link commands

Status: implemented v1
Updated: 2026-08-14

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

## Versioning rule

- Changes within a major `schemaVersion` are additive only — consumers must
  ignore unknown fields.
- A breaking change (removing, renaming, or retyping a field) bumps
  `schemaVersion`.
- Envelope versions are independent of the MCP API version and the archive
  format version (see docs/LOCAL_API.md).

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

## Won-stage rule

`link_job` is allowed only while the opportunity sits in a stage of kind
`won`; any other stage returns `validation_failed`
(`opportunity_not_won`) with a clear message. Quotes have no stage
restriction — quoting happens throughout the pipeline.
