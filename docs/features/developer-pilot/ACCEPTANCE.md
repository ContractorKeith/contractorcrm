# Developer pilot acceptance

Status: automated and installed clean candidate gates passed; merge pending.
Date: 2026-09-27, America/New_York.

## Automated evidence

Luna completed TypeScript, production frontend build, 206 frontend tests across
28 files, Rust formatting and Clippy with warnings denied. Full Rust tests:
336 passing, zero failing, two ignored (separate cross-app handoff and the
existing 10,000-contact scale test). Focused suites include 12 attachment,
39 portable-archive and 15 schema-contract tests.

Primary reviewed the complete implementation and regression diffs. Independent
Luna review identified missing saved-view, metadata and handoff-reference checks;
the corrected logic passed a subsequent focused review, including legitimate
archived metadata lifecycle history.

Primary separately ran the newly compiled ignored handoff test with the current
Project importer binary. It exported a won CRM opportunity, created a Project
job, linked it back, repeated import in another process and re-exported after
linking. All repeats returned the same Project job ID and original creation
timestamp. The cross-app test passed.

## Installed workflow evidence

The isolated ContractorCRM Pilot app launched with AI off and read-only agent
access. Created Synthetic Pilot Builder, a consulting opportunity and a dated
inspection follow-up. Contact CSV export contained the expected name, email and
stable external ID. A PDF attachment opened in Preview from the Pilot attachment
directory. An inert .command attachment was revealed in Finder with an explicit
status message and was not executed.

Exported a portable archive with 19 records and two attachments. Added a second
contact and verified it survived restart. Imported the archive through the native
UI, which created a pre-import safety backup; after restart only the original
contact and follow-up remained. Both restored attachment hashes match the archive.

A second archive with a recomputed valid checksum but an unknown contact kind was
rejected during preview with a field-specific application-validation error. The
replace button stayed disabled. Restarted and read the original contact normally.

## Pinned artifact

Source: `42ff770ad2281d4f07425bf69fda5b542765a399`, clean worktree at build.
Version 0.1.0; macOS 26.6.2; aarch64-apple-darwin.
Archive: `ContractorCRM-Pilot-42ff770ad228.zip`.
SHA-256: `2df6237b91d902c6a468211b44b93961d4f3447bb670d4026dcd1371b559d152`.

Installed this exact clean build in ~/Applications and verified its complete
ad-hoc signature. Reopened the restored contact and dated follow-up successfully.
Synthetic records and the archive are retained separately from the fresh working
Pilot profile in the Downloads handoff folder. Subsequent evidence-only commits
do not change the runtime represented by this source SHA. Integration is tracked
in PR #73 and issue #71.

## Limits

Issue #46 remains open: entry/size caps and moved attachment buffers reduce
memory pressure, but archive contents still remain in memory. Pilot imports
are restricted to trusted self-created archives. Documents need their own
backup plan. PDF/raster signature screening is not a malware scan.
