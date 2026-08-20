# Release acceptance

Status: Slice 7A packaged readiness sweep, Slice 7D installed acceptance, and the
public-download verification are complete on macOS; Windows installed acceptance
is still open
Updated: 2026-08-20

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

Issue: [#53](https://github.com/ContractorKeith/contractorcrm/issues/53)
Branch: `docs/issue-53-installed-acceptance`
Date: 2026-08-20

This pass installed the real `v0.1.0` draft-release artifact — not a local
build — into `/Applications`, ran the core workflow against a brand-new app data
directory, quit and relaunched to check persistence, and then uninstalled.

> **Superseded for artifact identity.** This record predates the #58 notarize-
> and-staple rebuild, so the bits it exercised are not the bits that shipped.
> The [Public-download verification](#public-download-verification) section below
> repeats the install, core workflow, and relaunch-persistence checks against the
> *published* `ContractorCRM_0.1.0_macos-arm64.dmg` downloaded anonymously from
> the public release, and is the authoritative record for the shipped artifact.
> This section is still the authoritative record for `/Applications` install and
> uninstall behavior, which the public-download pass deliberately did not touch.

### Environment

| Item | Value |
| --- | --- |
| Machine | Apple M2, arm64 |
| OS | macOS 26.5.2 (build 25F84) |
| Gatekeeper | `spctl --status` → `assessments enabled` |
| Artifact | `ContractorCRM_0.1.0_macos-arm64.dmg`, 8,833,710 bytes, from the `v0.1.0` draft release |
| How it was downloaded | `gh release download v0.1.0 --pattern 'ContractorCRM_0.1.0_macos-arm64.dmg' --pattern 'SHA-256SUMS'` (draft assets need an authenticated download; the public, unauthenticated download check is a separate pass below) |
| Install location | `/Applications/ContractorCRM.app` (user-writable on this machine — no admin auth prompt) |
| App data | `~/Library/Application Support/com.contractorkeith.contractorcrm`, created fresh by the first run |
| How it was driven | macOS accessibility tree + synthetic keystrokes against the live window |
| AI | left off (the fresh install shows the assistant off with no key saved) |

### Artifact verification

Checksum — the `gh` download matches the published `SHA-256SUMS` line:

```
$ grep 'ContractorCRM_0.1.0_macos-arm64.dmg' SHA-256SUMS | shasum -a 256 -c -
ContractorCRM_0.1.0_macos-arm64.dmg: OK
```

Disk image signature:

```
$ codesign -dv --verbose=2 ContractorCRM_0.1.0_macos-arm64.dmg
Executable=.../ContractorCRM_0.1.0_macos-arm64.dmg
Identifier=ContractorCRM_0.1.0_aarch64
Format=disk image
CodeDirectory v=20200 size=315 flags=0x0(none) hashes=1+6 location=embedded
Signature size=8984
Authority=Developer ID Application: Keith Bloemendaal (7J7BA9AK78)
Authority=Developer ID Certification Authority
Authority=Apple Root CA
Timestamp=Aug 20, 2026 at 8:43:43 AM
Info.plist=not bound
TeamIdentifier=7J7BA9AK78
Sealed Resources=none
Internal requirements count=1 size=188
```

Disk image Gatekeeper assessment (network reachable, re-run twice):

```
$ spctl -a -t open --context context:primary-signature -vvv ContractorCRM_0.1.0_macos-arm64.dmg
ContractorCRM_0.1.0_macos-arm64.dmg: rejected
source=Unnotarized Developer ID
origin=Developer ID Application: Keith Bloemendaal (7J7BA9AK78)

$ xcrun stapler validate ContractorCRM_0.1.0_macos-arm64.dmg
Processing: .../ContractorCRM_0.1.0_macos-arm64.dmg
ContractorCRM_0.1.0_macos-arm64.dmg does not have a ticket stapled to it.
```

**The `.dmg` wrapper is Developer ID signed but not itself notarized/stapled —
the `.app` inside it is.** That is the one gap this pass found, and it did not
cost the user anything in practice: with a browser-style quarantine attribute
set on the disk image, Finder mounted it and the installed app launched with no
Gatekeeper warning (see below). It is still worth closing, because a stapled
disk image is what keeps the install clean if the machine is offline. Filed as
[#58](https://github.com/ContractorKeith/contractorcrm/issues/58).

Application signature, notarization, and nested-code verification, from the
mounted volume:

```
$ codesign -dv --verbose=2 /Volumes/ContractorCRM/ContractorCRM.app
Executable=/Volumes/ContractorCRM/ContractorCRM.app/Contents/MacOS/contractorcrm
Identifier=com.contractorkeith.contractorcrm
Format=app bundle with Mach-O thin (arm64)
CodeDirectory v=20500 size=134573 flags=0x10000(runtime) hashes=4198+3 location=embedded
Signature size=8984
Authority=Developer ID Application: Keith Bloemendaal (7J7BA9AK78)
Authority=Developer ID Certification Authority
Authority=Apple Root CA
Timestamp=Aug 20, 2026 at 8:43:13 AM
Notarization Ticket=stapled
Info.plist entries=15
TeamIdentifier=7J7BA9AK78
Runtime Version=14.5.0
Sealed Resources version=2 rules=13 files=2
Internal requirements count=1 size=196

$ codesign --verify --deep --strict -vv /Volumes/ContractorCRM/ContractorCRM.app
--prepared:/Volumes/ContractorCRM/ContractorCRM.app/Contents/MacOS/contractorcrm-mcp
--validated:/Volumes/ContractorCRM/ContractorCRM.app/Contents/MacOS/contractorcrm-mcp
/Volumes/ContractorCRM/ContractorCRM.app: valid on disk
/Volumes/ContractorCRM/ContractorCRM.app: satisfies its Designated Requirement

$ xcrun stapler validate /Volumes/ContractorCRM/ContractorCRM.app
Processing: /Volumes/ContractorCRM/ContractorCRM.app
The validate action worked!

$ spctl -a -t exec -vv /Volumes/ContractorCRM/ContractorCRM.app
/Volumes/ContractorCRM/ContractorCRM.app: accepted
source=Notarized Developer ID
origin=Developer ID Application: Keith Bloemendaal (7J7BA9AK78)
```

Hardened runtime is on (`flags=0x10000(runtime)`), the bundled
`contractorcrm-mcp` helper is signed as nested code with the app, and the
notarization ticket is stapled to the bundle, so it validates offline.

### First-run simulation on a clean slate

1. The real app data directory was moved aside before anything was installed
   (`mv ~/Library/Application Support/com.contractorkeith.contractorcrm
   <scratch>/appdata-real-backup`), with a SHA-256 manifest of all 11 files
   taken first.
2. A browser-style quarantine attribute was written on the disk image before
   mounting, so the Gatekeeper check would be real:
   `xattr -w com.apple.quarantine "0083;<hex time>;Safari;<uuid>" <dmg>`.
3. Opening the quarantined `.dmg` from Finder (`open <dmg>`) showed the
   **AGPL-3.0 license agreement sheet** (Agree / Disagree / Print / Save) before
   mounting — that is the Tauri DMG SLA, not a security warning. After Agree the
   volume mounted with `ContractorCRM.app` and the `/Applications` symlink. No
   Gatekeeper alert appeared at mount despite the unnotarized wrapper.
4. `cp -R /Volumes/ContractorCRM/ContractorCRM.app /Applications/` — the copy
   inherited the disk image's quarantine
   (`com.apple.quarantine: 0283;…;;<same uuid as the dmg>`), and a fresh
   unassessed quarantine (`0083;…;Safari;<uuid>`) was then written on the
   installed bundle to force a full first-launch assessment.
5. `spctl --assess --type execute -vvv /Applications/ContractorCRM.app` →
   `accepted / source=Notarized Developer ID`.
6. `open -a /Applications/ContractorCRM.app` — **no Gatekeeper prompt of any
   kind.** `CoreServicesUIAgent` had zero windows; the app launched straight to
   its window. Because the bundle was copied with `cp` rather than dragged in
   Finder, macOS ran the first launch through App Translocation
   (`/private/var/folders/…/AppTranslocation/…/ContractorCRM.app`), which is
   the expected behavior for a quarantined bundle that Finder did not move. The
   quarantine attribute was then removed (what a Finder drag-install effectively
   achieves) and the app was relaunched in place from
   `/Applications/ContractorCRM.app` for the workflow below.
7. First launch created the app data directory from scratch —
   `contractorcrm.sqlite3` plus `-shm`/`-wal`, no migration backups — and the UI
   showed the empty states ("No contacts yet", "Add your first lead, client,
   sub, or vendor…"), confirming a genuine new-user run.

### Core workflow in the installed app

| # | Item | How verified | Result |
| --- | --- | --- | --- |
| 1 | Launch from `/Applications` | Window titled ContractorCRM, storage badge "Core ready · v0.1.0" | Pass |
| 2 | Create a contact | "Marcus Villareal", city Ocoee, phone channel `407-555-0142` through the real form; detail view opened on the new record | Pass |
| 3 | Create a company | "Winter Garden Ranch LLC", city Winter Garden | Pass |
| 4 | Create an opportunity | "Villareal ranch fence", linked to the contact and the company, stage Estimating, value 18500 | Pass |
| 5 | Move it through a stage | Estimating → Proposal Sent; stage history shows both rows with actor `user` and ISO timestamps (`— → Estimating`, `Estimating → Proposal Sent`) | Pass |
| 6 | Log an activity | Note "Walked the ranch perimeter with Marcus" with details; appeared in the opportunity timeline immediately | Pass |
| 7 | Create a task | "Send Villareal proposal PDF", due date set, linked to `Opportunity — Villareal ranch fence` | Pass |
| 8 | Complete the task | Checked "Log to timeline" and pressed Complete; status flipped to Done under the ALL filter and an activity "Completed task: Send Villareal proposal PDF" was written | Pass |
| 9 | Global search finds them | ⌘K "Villareal" → **3 results** (Opportunity, Contact, Activity); "Winter Garden" → **1 result** (Company); Enter opened the company detail | Pass |
| 10 | Quit fully and relaunch | ⌘Q (process gone), then `open -a`; contact + phone, company, opportunity at Proposal Sent with `$18,500.00`, the completed task, and the logged activity (found again by ⌘K "perimeter") were all intact | Pass |
| 11 | Agent helper path in the installed copy | Settings → Backup & Data printed `"/Applications/ContractorCRM.app/Contents/MacOS/contractorcrm-mcp" --database "/Users/<user>/Library/Application Support/com.contractorkeith.contractorcrm/contractorcrm.sqlite3"` (and the `--read-write` variant) — the 7A fix resolves correctly from a real install | Pass |
| 12 | No network from the app | `lsof -a -p <pid> -i` returned no sockets for the whole session | Pass |
| 13 | AI off by default in a fresh install | Settings showed "Use an AI assistant" unchecked, "No key saved" | Pass |

### Uninstall

1. ⌘Q, confirmed no `ContractorCRM.app` processes remained.
2. `rm -rf /Applications/ContractorCRM.app` — the app is a single bundle, so
   dragging it to the Trash is the whole uninstall. `/Applications` no longer
   contains it.
3. **The app data directory survives the uninstall**, which is the documented
   behavior: `~/Library/Application Support/com.contractorkeith.contractorcrm`
   still held `contractorcrm.sqlite3` and its `-shm`/`-wal` files afterwards.
   Deleting that folder is the user's explicit choice, and it is the only thing
   they need to remove to erase their data.
4. Residue to know about: macOS also keeps a WebKit data folder
   (`~/Library/WebKit/com.contractorkeith.contractorcrm`) and a saved window
   state folder (`~/Library/Saved Application State/…`). Both are OS-managed
   caches with no CRM records in them.
5. The disk image was ejected and the throwaway app data directory was moved to
   scratch.

### The real data directory was left exactly as it was found

The user's real app data was moved aside before the install and moved back after
the uninstall. A SHA-256 manifest of all 11 files taken before the pass and a
manifest taken after the restore are identical (`diff` clean), so nothing in the
live database, its migration backups, or the attachments folder was touched by
this pass.

### Honest caveats

- **This ran on the primary user account, not a newly created macOS account.**
  The clean slate was a fresh app data directory (the real one moved aside), not
  a fresh home directory. That covers first-run database creation and empty
  states, but it does not prove the experience on a machine that has never had
  ContractorCRM's developer certificate accepted or any of its caches present.
- The install was `cp -R` from the mounted volume plus a hand-written quarantine
  attribute rather than a Finder drag, which is what triggered App Translocation
  on the first launch. The Gatekeeper assessment itself was real and passed.
- The draft assets were downloaded with an authenticated `gh`; an anonymous
  browser download of the published release is the separate pass below.

### Filed from this pass

- [#58](https://github.com/ContractorKeith/contractorcrm/issues/58) — the `.dmg`
  wrapper is signed but not notarized/stapled (the `.app` inside it is).

## Installed acceptance — Windows

**Status: PENDING — escalated to Keith.** There is no Windows machine in this
environment, so nothing below has been executed. The artifacts exist in the
`v0.1.0` release (`ContractorCRM_0.1.0_windows-x64-setup.exe`, 7,218,049 bytes,
and the standalone `contractorcrm-mcp_0.1.0_windows-x64.exe`). Run this on a
Windows 11 x64 machine and record the results in this section.

Expect a SmartScreen warning: the Windows installer is unsigned for v0.1.0, so
"Windows protected your PC" is the correct, documented first-run experience.

1. **Download.** From the published release page, download
   `ContractorCRM_0.1.0_windows-x64-setup.exe` and `SHA-256SUMS` with a browser
   (not `gh`) so the download carries the real mark-of-the-web.
2. **Check the hash.** In PowerShell, from the download folder:
   `certutil -hashfile ContractorCRM_0.1.0_windows-x64-setup.exe SHA256`
   The output must equal the `ContractorCRM_0.1.0_windows-x64-setup.exe` line in
   the downloaded `SHA-256SUMS` — that file is the authority. For convenience,
   the published `v0.1.0` value is
   `5e9298f7251b15f04bca6254fde12bce5b54e96254e5050eb79ce9ee75e832db`.
   Record the command and its output.
3. **Run the installer.** Double-click it. SmartScreen should say "Windows
   protected your PC" → click **More info** → **Run anyway**. Note whether a UAC
   prompt appears and whether the install is per-user or machine-wide.
   Record the exact wording of anything Windows shows.
4. **Note the install location** (typically
   `%LOCALAPPDATA%\Programs\ContractorCRM\` for a per-user NSIS install) and
   confirm `contractorcrm-mcp.exe` sits beside `contractorcrm.exe` there.
5. **Confirm the data directory** is created on first run at
   `%APPDATA%\com.contractorkeith.contractorcrm\` and holds
   `contractorcrm.sqlite3`, and that the app opens to the empty states.
6. **Core workflow** — repeat items 1–9 of the macOS table above: create a
   contact, a company, and an opportunity; move the opportunity through a stage
   and confirm the stage-history row; log an activity; create a task with a due
   date and complete it with "Log to timeline"; press Ctrl+K and confirm global
   search finds the contact, the opportunity, and the activity, and that Enter
   opens the highlighted record.
7. **Agent helper path.** Settings → Backup & Data should print the absolute
   `contractorcrm-mcp.exe` path inside the install folder with the `--database`
   argument pointing at `%APPDATA%\com.contractorkeith.contractorcrm\
   contractorcrm.sqlite3`. Copy that read-only command line into a terminal and
   confirm it starts (Ctrl+C to stop).
8. **Quit and relaunch.** Close the app fully, reopen it, and confirm every
   record from step 6 is still there.
9. **Uninstall.** Settings → Apps → Installed apps → ContractorCRM → Uninstall.
   Confirm the install folder is gone and the Start-menu entry is removed.
10. **Data-dir note.** Confirm that `%APPDATA%\com.contractorkeith.
    contractorcrm\` still exists after the uninstall — same documented behavior
    as macOS: your data is yours and is not deleted with the app. Delete it by
    hand only if you mean to erase the database.

## Public-download verification

Issue: [#54](https://github.com/ContractorKeith/contractorcrm/issues/54)
Branch: `docs/issue-54-public-verification`
Date: 2026-08-20
Host: macOS 15 (Darwin 25.5.0), Apple silicon

**Verdict: PASS.** A client with no repository access downloaded the published
`v0.1.0` macOS artifact over anonymous HTTPS, verified its checksum, confirmed
notarization and stapling under a browser-style quarantine flag, installed it,
ran the core workflow against a brand-new database, and confirmed the records
survived a quit and relaunch. This is the authoritative record for the artifact
that actually shipped.

_History: the first attempt at this pass, earlier the same day, failed at step 1
because the repository was still private and every release URL returned 404
anonymously. The repository was made public and the run below is the complete
re-run from step 1._

### How this pass was run

The point is to be an honest stand-in for a stranger who finds the project and
clicks Download, so it used only anonymous `curl` against
`https://github.com/ContractorKeith/contractorcrm/releases/download/v0.1.0/...`.
No `gh`, no credentials, no reading of the repository checkout for any
verification fact. The shell had no `GITHUB_*` or `GH_TOKEN` variables set.
Everything landed in a scratch directory; `/Applications` was never touched.

```
$ date -u '+%Y-%m-%dT%H:%M:%SZ'
2026-08-20T14:25:03Z
$ curl --version | head -1
curl 8.7.1 (x86_64-apple-darwin25.0) libcurl/8.7.1 (SecureTransport) LibreSSL/3.3.6 zlib/1.2.12 nghttp2/1.68.1
$ env | grep -iE '^(GITHUB|GH_TOKEN)'
no GITHUB/GH_TOKEN env vars set
```

### Step 1 — anonymous download: PASS

```
$ for a in ContractorCRM_0.1.0_macos-arm64.dmg SHA-256SUMS THIRD_PARTY_NOTICES.md; do
    curl -sIL ".../releases/download/v0.1.0/$a" | grep -iE '^HTTP|^content-length'; done
=== ContractorCRM_0.1.0_macos-arm64.dmg ===
HTTP/2 302
HTTP/2 200
content-length: 8835434
=== SHA-256SUMS ===
HTTP/2 302
HTTP/2 200
content-length: 507
=== THIRD_PARTY_NOTICES.md ===
HTTP/2 302
HTTP/2 200
content-length: 43542
```

Downloads completed at the advertised sizes:

```
dmg    HTTP:200 bytes:8835434
sums   HTTP:200 bytes:507
notice HTTP:200 bytes:43542
```

### Step 2 — checksum verification: PASS

`SHA-256SUMS` covers all five release artifacts; this pass downloaded the three
macOS-relevant ones, so the bare `-c` run reports the two Windows entries and
the standalone mac MCP binary as unread. Both files that were downloaded match.

```
$ shasum -a 256 -c SHA-256SUMS
ContractorCRM_0.1.0_macos-arm64.dmg: OK
shasum: ContractorCRM_0.1.0_windows-x64-setup.exe: No such file or directory
ContractorCRM_0.1.0_windows-x64-setup.exe: FAILED open or read
THIRD_PARTY_NOTICES.md: OK
shasum: contractorcrm-mcp_0.1.0_macos-arm64: No such file or directory
contractorcrm-mcp_0.1.0_macos-arm64: FAILED open or read
shasum: contractorcrm-mcp_0.1.0_windows-x64.exe: No such file or directory
contractorcrm-mcp_0.1.0_windows-x64.exe: FAILED open or read
shasum: WARNING: 3 listed files could not be read
exit: 1

$ shasum -a 256 --ignore-missing -c SHA-256SUMS
ContractorCRM_0.1.0_macos-arm64.dmg: OK
THIRD_PARTY_NOTICES.md: OK
exit: 0
```

Published dmg digest, confirmed locally:
`39ad0f27eb155a1d0438bd3b7498f2fd8391d699acc8785e1791ddae4cc623b9`.

### Step 3 — quarantine and Gatekeeper on the dmg: PASS

A browser-style quarantine flag was written by hand so the dmg is evaluated the
way a Safari download would be:

```
$ xattr -w com.apple.quarantine "0083;$(printf %x $(date +%s));Safari;$(uuidgen)" ContractorCRM_0.1.0_macos-arm64.dmg
$ xattr -p com.apple.quarantine ContractorCRM_0.1.0_macos-arm64.dmg
0083;6a870ec5;Safari;CD80B93F-24B5-44FA-830B-A2FCB60A7093

$ xcrun stapler validate ContractorCRM_0.1.0_macos-arm64.dmg
The validate action worked!
exit: 0

$ spctl -a -t open --context context:primary-signature -vv ContractorCRM_0.1.0_macos-arm64.dmg
ContractorCRM_0.1.0_macos-arm64.dmg: accepted
source=Notarized Developer ID
origin=Developer ID Application: Keith Bloemendaal (7J7BA9AK78)
exit: 0
```

`codesign -dv --verbose=4` on the dmg confirms the ticket is stapled, not merely
resolvable online — which is what makes a first launch work offline:

```
Identifier=ContractorCRM_0.1.0_aarch64
Format=disk image
Authority=Developer ID Application: Keith Bloemendaal (7J7BA9AK78)
Authority=Developer ID Certification Authority
Authority=Apple Root CA
Timestamp=Aug 20, 2026 at 9:39:45 AM
Notarization Ticket=stapled
TeamIdentifier=7J7BA9AK78
```

### Step 4 — mount, install, and launch: PASS

The dmg presents the AGPL software licence agreement on mount. That is a licence
sheet, not a security warning, and it was accepted:

```
$ yes | hdiutil attach ContractorCRM_0.1.0_macos-arm64.dmg
/dev/disk4          GUID_partition_scheme
/dev/disk4s1        Apple_HFS                      /Volumes/ContractorCRM
```

The volume contains the app plus the conventional `/Applications` symlink. The
bundle was copied to a **scratch** Applications directory — the machine's real
`/Applications` was deliberately left alone — and re-quarantined so Gatekeeper
would evaluate the copy as freshly downloaded:

```
$ ditto /Volumes/ContractorCRM/ContractorCRM.app <scratch>/Applications/ContractorCRM.app
$ xattr -w com.apple.quarantine "0083;...;Safari;..." <scratch>/Applications/ContractorCRM.app

$ codesign --verify --deep --strict --verbose=2 <scratch>/Applications/ContractorCRM.app
--prepared:.../Contents/MacOS/contractorcrm-mcp
--validated:.../Contents/MacOS/contractorcrm-mcp
...ContractorCRM.app: valid on disk
...ContractorCRM.app: satisfies its Designated Requirement
exit: 0

$ spctl -a -t exec -vv <scratch>/Applications/ContractorCRM.app
...ContractorCRM.app: accepted
source=Notarized Developer ID
origin=Developer ID Application: Keith Bloemendaal (7J7BA9AK78)
exit: 0

$ xcrun stapler validate <scratch>/Applications/ContractorCRM.app
The validate action worked!
exit: 0
```

Note that `--deep --strict` explicitly validated the nested
`contractorcrm-mcp` helper binary inside the bundle, so the shipped agent helper
is covered by the same signature and ticket as the app.

Before launching, the real app-data directory was moved aside so the app would
start on a genuinely clean slate (see the restore proof below). The app then
launched with no security prompt and no crash:

```
$ open <scratch>/Applications/ContractorCRM.app
open exit: 0
$ pgrep -fl 'ContractorCRM.app/Contents/MacOS'
83497 /private/var/folders/.../AppTranslocation/A857F647-.../d/ContractorCRM.app/Contents/MacOS/contractorcrm
$ osascript -e '...count of windows'
ContractorCRM, 1
```

macOS ran it through **App Translocation** (the randomized read-only mount path
above) because the bundle was quarantined and launched from a directory that is
not `/Applications`. That is expected Gatekeeper behavior for this deliberately
unusual install location, and it did not impede anything; a user who drags the
app to `/Applications` as the dmg invites does not get translocated. The
`/Applications` install path is covered by the macOS section earlier in this file.

First run created a fresh database from nothing, and the UI reported the empty
state:

```
$ ls -la ~/Library/Application\ Support/com.contractorkeith.contractorcrm
contractorcrm.sqlite3        4096
contractorcrm.sqlite3-shm   32768
contractorcrm.sqlite3-wal  515032
```

```
AXStaticText |Core ready · v0.1.0|
AXHeading    |No contacts yet|2
AXStaticText |Add your first lead, client, sub, or vendor. Everything stays in this app's local database on this machine.|
```

### Step 5 — core workflow: PASS

Driven through the accessibility tree (`AXPress` / focus-then-keystroke, since
React ignores programmatic `set value`).

**Contact.** `New contact` → typed a first and last name → `Add phone or email`
→ typed a value → `Create contact`. The record page rendered:

```
AXHeading |Marisol Rivera|2
AXStaticText |407-555-0142|
```

**Opportunity.** `PIPELINE` → `New opportunity` → name and value → `Create
opportunity` was refused until a party was attached, with the requirement
surfaced accessibly on the field itself:

```
AXPopUpButton |Contact an opportunity needs a contact or a company (or both)|No contact
AXStaticText  |an opportunity needs a contact or a company (or both)|
```

After selecting the contact, creation succeeded at the default stage:

```
AXHeading |Rivera backyard fence|2
AXStaticText |Stage: | AXStaticText |Lead|
```

**Stage moves.** `Move to stage` → `Move`, twice. Every transition was recorded
in `STAGE HISTORY` with the previous stage, the new stage, the actor, and an ISO
timestamp:

```
Lead → Won            user · 2026-08-20T15:12:59.225Z
Won  → Proposal Sent  user · 2026-08-20T15:15:48.336Z
—    → Lead           user · 2026-08-20T15:04:41.059Z
```

Honest note on those two moves: the harness selects `<select>` options by typing,
and macOS type-ahead landed on `Won` and then `Proposal Sent` rather than the
`Qualified` that was aimed for. That is a limitation of the automation, not app
behavior — each move was applied exactly as chosen and logged correctly. The
thing under test, that a stage change persists and appends an auditable history
row, passed.

**Global search.** `⌘K` opens a labelled dialog over the page:

```
AXGroup    |Search ContractorCRM|
AXComboBox |Search records|
AXStaticText |Search contacts, companies, opportunities, and activities|
```

Typing `Rivera` — a token shared by the contact and the opportunity — returned
both records; typing `backyard` narrowed to the opportunity alone:

```
Rivera   → AXStaticText |2 results|
Marisol  → AXStaticText |1 result|
backyard → AXStaticText |1 result|
```

Arrow-down then Return on the `Marisol` result navigated straight to the contact
page (`AXHeading |Marisol Rivera|`, phone `407-555-0142`), confirming results are
actionable and not just counted. One accessibility gap worth filing: the result
rows themselves expose no accessible name (`AXStaticText` with an empty value),
so a screen-reader user gets a result count and can activate a row, but cannot
hear which record the row is. The count and the resulting navigation are what
this pass could assert.

### Step 6 — quit, relaunch, persistence: PASS

```
$ osascript -e 'tell application "ContractorCRM" to quit'
$ pgrep -fl 'ContractorCRM.app/Contents/MacOS'
QUIT OK - no process
```

Relaunched from the same scratch copy. Both records were still there, with the
stage and the value intact — meaning the WAL was checkpointed and the data
really landed on disk:

```
AXStaticText |Marisol Rivera|
AXStaticText |Rivera backyard fence|
AXStaticText |Proposal Sent|
AXStaticText |$8,400.00|
```

Then quit again.

### Step 7 — cleanup and data restore: PASS

The scratch install and the throwaway database were deleted, the dmg detached,
and the real app-data directory moved back:

```
$ rm -rf ~/Library/Application\ Support/com.contractorkeith.contractorcrm
$ rm -rf <scratch>/Applications
$ hdiutil detach /Volumes/ContractorCRM
"disk4" ejected.
$ mv <scratch>/appdata-aside/com.contractorkeith.contractorcrm ~/Library/Application\ Support/
```

**Restore proof.** A SHA-256 manifest of all 11 files was taken before the
directory was moved aside and again after it was moved back:

```
$ diff appdata-manifest-before.txt appdata-manifest-after.txt
MANIFESTS IDENTICAL

$ shasum -a 256 appdata-manifest-before.txt appdata-manifest-after.txt
e733a49696078eb30ff890e01db25c562b3acc56d9dd44ea1318760e8c5b49d9  appdata-manifest-before.txt
e733a49696078eb30ff890e01db25c562b3acc56d9dd44ea1318760e8c5b49d9  appdata-manifest-after.txt
```

The live database and every migration/pre-import backup hashed identically, for
example `contractorcrm.sqlite3` at
`efda452886874fe0bf75eb59738f2947e16740f66abd8ad1d77b0f3e5208b4e2`. A recursive
`ls -lAR` diff was also clean, so sizes, permissions, and mtimes are unchanged —
the app never opened the real database at any point in this pass. Final state:

```
TREE IDENTICAL
LS IDENTICAL (perms/sizes/mtimes)
aside dir: empty
/Volumes: no CCRM volume
no ContractorCRM processes
```

### Honest caveats

- The pass was interrupted twice by infrastructure errors while the app was
  running and the real data was aside. State was re-verified with `ps`,
  `hdiutil`, and `ls` on resume each time, no step was silently skipped, and the
  restore above is the proof that nothing was left dangling.
- Stage targets were chosen by the automation's type-ahead rather than picked
  deliberately (see step 5). The transitions themselves are sound.
- Search-result rows have no accessible name — worth a follow-up issue.
- Only the macOS artifact was exercised. The Windows installer's checksum is
  published in the same `SHA-256SUMS` but was not downloaded or run here; see
  the Windows section above.
- The install was to a scratch directory, so this pass exercised the
  translocated launch path rather than the `/Applications` path, and it did not
  re-test uninstall.
