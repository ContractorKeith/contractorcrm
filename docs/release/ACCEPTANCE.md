# Release acceptance

Status: Slice 7A packaged readiness sweep complete on macOS; installed and
public-download passes still open
Updated: 2026-08-19

This file is the running record of what has been verified in real, packaged
builds of ContractorCRM — not in `npm run tauri dev`. Each section names the
environment, the artifact, and how every line item was checked.

## Packaged readiness sweep (7A)

Issue: [#50](https://github.com/ContractorKeith/contractorcrm/issues/50)
Branch: `chore/issue-50-release-readiness`

### Environment

| Item | Value |
| --- | --- |
| Machine | Apple silicon Mac, arm64 |
| OS | macOS 26.5.2 (build 25F84) |
| Rust | 1.97.1 (Homebrew) |
| Node | v26.7.0 |
| Build command | `npm run tauri build` (full bundle) |
| App bundle | `src-tauri/target/release/bundle/macos/ContractorCRM.app` — 23 MB |
| Disk image | `src-tauri/target/release/bundle/dmg/ContractorCRM_0.1.0_aarch64.dmg` — 8.4 MB |
| Signing | ad-hoc / linker-signed only; Developer ID signing and notarization are later 7-series work |
| How it was run | the `.app` was copied out of `target/` to a scratch directory ("installed") and launched from there; the app used the real data directory `~/Library/Application Support/com.contractorkeith.contractorcrm` |
| How it was driven | macOS accessibility tree + synthetic keystrokes against the live window, and the bundled `contractorcrm-mcp` binary over stdio for the agent surface |
| AI | left off for the whole sweep, except for reading the settings surface itself |

### Checklist

| # | Item | How verified | Result |
| --- | --- | --- | --- |
| 1 | App launches from the package | Copied `.app` to a scratch dir, `open -a`; window titled ContractorCRM, storage badge "Core ready · v0.1.0"; app menu has About/Hide/Quit | Pass (after fix 1 — the first bundle shipped the wrong executable) |
| 2 | Data directory and migrations | First packaged launch migrated the existing database forward and wrote `contractorcrm.sqlite3.pre-migration-v7…v11.bak` beside it in the app data dir | Pass |
| 3 | Contacts create / edit | Created "Rosa Delgado" with a phone channel through the real form; reopened, edited city and notes, saved; detail view showed the new values | Pass |
| 4 | Companies create / archive / unarchive | Created "Lakeview Builders"; Archive showed the ARCHIVED badge and the control flipped to Unarchive; Unarchive cleared it | Pass |
| 5 | Relaunch persistence | Hard-killed the app (`pkill`, no graceful quit), relaunched the package; contacts, company, opportunity, stage history, task, and attachment were all intact | Pass |
| 6 | Pipeline stage moves | Created "Delgado backyard fence" at Estimating, moved to Proposal Sent, then Lost, then Won; each move appended a stage-history row with actor `user` and a timestamp | Pass |
| 7 | Lost-reason rule | Chose Lost with no reason and pressed Move: refused inline with "Select a lost reason before moving to a lost stage.", stage unchanged; with reason "Price" the move succeeded and the history line shows `· Price` | Pass |
| 8 | Activity history and timeline | Logged a note on the company ("Kickoff call about fence scope"); it appeared in the timeline with edit/delete controls | Pass |
| 9 | Tasks | Created a task with a due date, reopened it, completed it with "Log to timeline"; it moved to Done under the ALL filter | Pass |
| 10 | Attention flags | Dropped the stale-lead threshold to 1 day and saved: the flag `Lead "Dana Whitfield" has had no activity in 3 day(s).` appeared with a jump-to-record button; restored the threshold to 21 and it cleared | Pass |
| 11 | Global search | ⌘K, typed "Delgado": 2 results (opportunity + contact); Down/Enter opened the contact detail | Pass |
| 12 | CSV export | Contacts → Export CSV… wrote 2 contacts; Pipeline → Export CSV… wrote 2 opportunities; both files have the documented headers and values | Pass |
| 13 | Backup + restore round trip | The portable archive round trip below is the user-facing path, and the import wrote `contractorcrm.sqlite3.pre-import-20260820T031933473Z.bak` — the same `backup_database` seam. There is no standalone backup/restore button in the package | Pass, with [#56](https://github.com/ContractorKeith/contractorcrm/issues/56) filed |
| 14 | Archive export + import round trip | Exported "21 files and 40 records" to a ZIP (manifest, 17 data files, the attachment bytes under `assets/`, plus the CSV mirrors), then imported the same file: preview listed the record counts, "Replace all data and import" completed, safety backup path reported, and every record was still there afterwards | Pass |
| 15 | Hand-off envelope export | Linked quote `Q-2026-0142` (tool `contractorproject`) on the won deal, exported the envelope to a chosen path: `schemaVersion 1`, `kind opportunity_handoff`, product/version, full opportunity with `quoteRef` | Pass |
| 16 | Attachments add / open / remove | Added `scope-notes.txt` through the native picker — stored as a managed copy at `<app data>/attachments/<id>/scope-notes.txt`; Open launched TextEdit; Remove (two-step confirm) deleted both the row and the file on disk | Pass (after fix 2 — open was refused in the first package) |
| 17 | AI-off surface | Settings shows the assistant off, all fields reachable, no prompts and no dead ends. `lsof -p <pid> -i` listed no network sockets and the login keychain has no ContractorCRM entry after the full sweep | Pass |
| 18 | MCP helper present in the distribution | `ContractorCRM.app/Contents/MacOS/contractorcrm-mcp` ships inside the bundle (6.5 MB) alongside the app executable | Pass |
| 19 | MCP helper runs against the packaged data | Ran the bundled binary with `--database "<app data>/contractorcrm.sqlite3"`: `initialize` reported `mode: read_only`, product 0.1.0, local API 1; `tools/list` returned the 25 read/draft tools; `search_records "Delgado"` returned the records created in the GUI | Pass |
| 20 | MCP read-only default refuses writes | `create_contact` over the read-only connection returned `isError: true` with kind `read_only`; a missing file and a non-ContractorCRM SQLite file are both refused on stderr with exit 1 | Pass |
| 21 | Disk image | `ContractorCRM_0.1.0_aarch64.dmg` mounts and contains `ContractorCRM.app` plus the `/Applications` symlink | Pass |

### Release-blocking fixes made in this sweep

1. **The bundle shipped the wrong executable.** The crate has three binaries, so
   packaging picked one by name order: `Contents/MacOS/contractorcrm-mcp` was
   the only executable in the bundle and `CFBundleExecutable` pointed at it, so
   the "app" was the stdio MCP server and the desktop app was absent.
   Fix: `default-run = "contractorcrm"` in `src-tauri/Cargo.toml:9`. After the
   fix the bundle carries both binaries and launches the desktop app.

2. **Opening an attachment was refused in the package.** `opener:default` does
   not include `open_path`, so `openPath()` rejected and the UI showed "The
   file could not be opened." Unit tests mock the plugin, so nothing caught it.
   Fix: `src-tauri/capabilities/default.json:11` now adds
   `opener:allow-open-path` scoped to `$APPDATA/attachments/**` — managed files
   open, nothing else on disk does.

3. **The agent onboarding command line was not runnable.** Settings printed
   `contractorcrm-mcp --database "…"`, but the helper ships inside the app
   bundle and is not on anyone's `PATH`. Fix: new read command
   `get_agent_helper_path` (`src-tauri/src/lib.rs:801`, registered at
   `src-tauri/src/lib.rs:132`, manifest entry `schemas/v1/local-api.json:72`,
   client method `src/api/client.ts:282`) resolves the helper next to the
   running executable, and `src/views/settings.tsx:263` prints the quoted
   absolute path in both command lines.

### MCP helper distribution decision

**The helper ships inside the application bundle, next to the app executable**
(`ContractorCRM.app/Contents/MacOS/contractorcrm-mcp`; on Windows,
`contractorcrm-mcp.exe` beside `contractorcrm.exe`), and Settings → Backup &
Data prints its absolute path.

Rationale:

- Cargo already builds every binary in the crate and the Tauri bundler already
  copies them into `Contents/MacOS`, so this needs no `externalBin` entry, no
  prebuilt artifact committed to the repo, and no chicken-and-egg build step
  (`externalBin` requires the file to exist before the app is compiled).
- `Contents/MacOS` is the correct home for a helper executable under macOS code
  signing and notarization — a second binary there is signed with the bundle,
  where an executable dropped into `Contents/Resources` is nested code that has
  to be handled separately.
- One artifact means the user installs one thing and the helper can never drift
  out of version with the app that wrote the database. A separately uploaded
  release artifact would have to be version-matched by hand.
- Nothing has to change in CI: whatever uploads the `.app`/`.dmg` uploads the
  helper with it.

### Filed, not fixed

- [#56](https://github.com/ContractorKeith/contractorcrm/issues/56) — backup and
  restore have no desktop surface (portable archive covers the v1 story).
- [#57](https://github.com/ContractorKeith/contractorcrm/issues/57) — native
  `<select>` record pickers ("Linked to") are keyboard-hostile for long lists.

### Gates

Run on the final tree, all green: `cargo fmt --check`, `cargo clippy
--all-targets -- -D warnings`, `cargo test --all-targets`, `npm run typecheck`,
`npm test`, `npm run build`, and `npm run tauri build`.

## Installed acceptance — macOS

_Not started._ To be filled by the pass that installs a signed, notarized build
from the `.dmg` into `/Applications` on a clean account and repeats the
checklist above.

## Installed acceptance — Windows

_Not started._ To be filled by the pass that installs the Windows package
(MSI/NSIS) on a Windows machine and repeats the checklist above, including the
`contractorcrm-mcp.exe` helper path shown in Settings.

## Public-download verification

_Not started._ To be filled by the pass that downloads the published release
artifacts from GitHub on a machine that never built the app, checks the
checksums and signatures, and confirms the app opens without Gatekeeper
warnings.
