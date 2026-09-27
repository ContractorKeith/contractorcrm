# Personal developer pilot

Status: implementation and acceptance in progress. No installed acceptance claimed.
Tracker: https://github.com/ContractorKeith/contractorcrm/issues/71

## Goal and boundary

Prepare ContractorCRM for Keith's single-operator consulting work with Google Drive Sheets maintained in parallel. Core work remains offline. AI stays off or read-only until its outputs have been reviewed. Public release and wider distribution remain separate milestones.

Use the Pilot app identifier and a separate data directory. Never load real client records into test fixtures, development profiles or issue attachments. Use separate company records/files where supported and keep an ID cross-reference for each consulting engagement. Source documents and receipts need separate backups.

## Work and integration

Primary agent owns the complete diff review, acceptance, documentation and git integration. GPT-6 Luna at medium thinking implements bounded issues without nested delegation. Send failed checks or review findings back for correction; rebase against current main when needed.

Integration order: CRM safety, Project recovery and retry-safe imports, Books reproducibility and acceptance. Then run the real CRM-to-Project envelope handoff and verify repeat import. Each app has its own branch, PR, tests and merge evidence. User-facing changes also update the website's canonical app documentation.

## Candidate build

```sh
npm ci
node scripts/build-pilot.mjs
```

The pilot identifier is `com.contractorkeith.contractorcrm.pilot`. On macOS its default data directory is `~/Library/Application Support/com.contractorkeith.contractorcrm.pilot`. Do not point development or an MCP helper at this directory while the app is running. Books' explicit environment override takes precedence if set; clear it for normal Pilot launch.

The script records exact source SHA, dirty state, app version, macOS version and artifact checksum. It stages a local copy and verifies an ad-hoc code signature. This does not establish Developer ID signing, notarization or public-release acceptance.

## Acceptance gates

- [ ] Required safety and recovery issues pass focused regression coverage.
- [ ] Repository frontend, Rust and applicable browser/version gates pass.
- [ ] Packaged Pilot app launches using the isolated data directory.
- [ ] Create realistic synthetic records, edit and restart; records persist.
- [ ] Export human-readable data and check it against the synthetic scenario.
- [ ] Create backup, change data, restore a copy, reopen and compare.
- [ ] Corrupt recovery input fails without destroying current usable data.
- [ ] Optional AI/model availability does not block ordinary work.
- [ ] Parent reviews full branch diff and canonical website docs.
- [ ] Scoped commits merged to main and pushed; artifact SHA and checks recorded.

Use `ACCEPTANCE.md` in this directory for evidence. Mark incomplete native steps pending. Keith's later client-data trial and business/accountant decisions cannot be inferred from synthetic engineering tests.

## First consulting week

Create separate records for the active house and other lots. Confirm completed work through the slab without inventing historical dates. Label the first schedule baseline as the takeover/remaining-work forecast. Keep dated contact follow-ups, a reviewed two-week schedule export and the cross-app ID register alongside Google Sheets. Compare Books reports to independently checked records; partial entries do not establish profitability.

After each work session, make application-consistent backups and copy completed backups and external documents off-machine. Keep live SQLite databases local. Record reproducible defects with app/source version, steps, expected and actual behavior, using synthetic examples.
