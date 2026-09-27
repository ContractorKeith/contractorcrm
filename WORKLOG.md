# Worklog

### 2026-09-27 America/New_York - Developer pilot planning and implementation

- Outcome: Created pilot tracker #71 and candidate task #72; reused #47/#49/#46 and #65. Safety implementation and archive review are in progress. Updated the real cross-app handoff test to require a stable job on repeated import and re-export. Incorporates PR #45's Books reference intent while preserving newer handoff documentation.
- Decision: Keith operates one local Mac pilot with Google Sheets parallel records. Separate pilot identifiers protect normal app data. Public release and client/accountant acceptance remain separate.
- Execution: GPT-6 Luna medium handles bounded tasks without nested delegation; primary reviews full diffs and owns integration, installed acceptance and shipping.
- Next: finish regression gates, package exact candidates, perform native recovery drills, record evidence and merge/push each scoped branch.


## 2026-09-27 — Native CRM pilot acceptance

Installed isolated app passed contact/opportunity/follow-up creation, CSV export,
PDF opening and executable-type reveal-only behavior with AI off. Native archive
round trip restored 19 records and two byte-identical attachments after a persisted
extra contact; a checksum-correct archive with invalid contact kind was rejected
before replacement, with the restored contact still readable after restart.
Automated gates and full implementation review passed. Next: clean pinned package,
provenance and PR integration.

## 2026-09-27 — Pinned CRM candidate

Clean source 42ff770ad2281d4f07425bf69fda5b542765a399 built, installed and passed strict ad-hoc
signature verification. The restored contact and dated follow-up remained readable.
Retained synthetic profile and backups separately in the Downloads pilot handoff.
Artifact hash is in ACCEPTANCE.md. PR #73 is pushed; CI/merge remains.

## 2026-09-27 — Pilot preparation complete

PR #73 merged as 974ea608f444887a966c565ce03b3f847a5e0de1 after all PR checks passed.
Main fast-forwarded; complete branch reviewed. Fresh Pilot profile verified empty,
retained ZIP independently extracted/signature-checked and matched to installed
runtime. Packaged Project importer passed cross-app retry/re-export acceptance.
Normal app data and pre-existing untracked backups preserved.
Next: Keith enters confirmed consulting records with Sheets in parallel; #46
streaming archive work remains separate from this bounded trusted-archive pilot.
