# Recovery: crashes, migrations, and damaged databases

How ContractorCRM protects a local database, what it does automatically, and what
to do by hand when something goes wrong. Companion to `docs/ARCHITECTURE.md`
(local-first data rules) and `docs/DATA_MODEL.md` (schema).

The contractor-facing version of this page lives in the docs site at
`src/content/docs/crm/import-export-backup.md` ("If your data won't open").

## The files on disk

One database per installation, in the OS application-data directory for
`com.contractorkeith.contractorcrm`:

| Platform | Directory |
| --- | --- |
| macOS | `~/Library/Application Support/com.contractorkeith.contractorcrm/` |
| Windows | `%APPDATA%\com.contractorkeith.contractorcrm\` (`C:\Users\<you>\AppData\Roaming\...`) |
| Linux | `~/.local/share/com.contractorkeith.contractorcrm/` |

What lives there:

- `contractorcrm.sqlite3` — the database. This is the file that matters.
- `contractorcrm.sqlite3-wal`, `-shm` — SQLite's write-ahead log and shared-memory
  index. Normal while the app runs; left behind after a crash and replayed on the
  next open. Never copy the database without them unless the app is closed.
- `contractorcrm.sqlite3.pre-migration-v<N>.bak` — automatic copy taken before
  schema version `N` is applied to an existing database.
- `contractorcrm.sqlite3.pre-restore-<stamp>.bak` — automatic copy taken before a
  restore replaces the live database.
- `contractorcrm.sqlite3.pre-import-<stamp>.bak` — automatic copy taken before a
  portable archive import replaces the live database.
- `attachments/` — managed attachment files, one directory per attachment id.
  Attachments are **not** inside the database and **not** inside a `.bak` file.

## Migration model

Forward-only. There is no "down" migration and there never will be; going back
means restoring a file.

- Migrations are an ordered list of `(version, sql)` in `src-tauri/src/storage.rs`.
  Old entries are never edited — schema changes are appended as a new version.
- `schema_migrations` records each applied version. Opening a database applies
  only the versions it does not already have, so reopening is a no-op.
- Each migration runs inside its own `BEGIN IMMEDIATE` transaction together with
  its `schema_migrations` insert. A migration either lands whole or not at all:
  if any statement fails, its schema changes **and** its ledger row roll back and
  the database stays at the previous version, openable and usable.
- Before applying a pending migration to a database that already existed on disk,
  the app writes `contractorcrm.sqlite3.pre-migration-v<N>.bak` (SQLite backup
  API, so it is a consistent copy). It is written once per version — an existing
  file is left alone — and it is written before the attempt, so a failed
  migration still leaves you a copy of exactly the state you were in.
- A backup whose schema version is **newer** than the running build is refused on
  restore. Update the app first; older backups migrate forward on open.

Covered by `src-tauri/tests/storage.rs`:
`a_failing_migration_leaves_no_schema_or_ledger_trace` and
`restoring_a_pre_migration_backup_returns_to_the_old_version_and_re_migrates`.

## What crash recovery gives you

Every write goes through the Rust application seam in one transaction — a
contact and its channels, its search projection, and its audit-log entry are one
atomic unit, never four separate writes. The database runs in WAL mode, so an
interrupted process (kill, panic, power loss, forced restart) leaves a WAL that
SQLite replays on the next open: committed transactions are there, an
in-flight transaction is gone entirely. There is no partial record and no
repair step for the user to run.

`src-tauri/tests/crash_recovery.rs` proves this against a real `SIGKILL`: a child
process writes through the application seam in a loop, is killed mid-write, and
the database is reopened five times over. Each time `PRAGMA integrity_check`
returns `ok`, the foreign-key check is clean, no record exists with only part of
its transaction, the committed sequence has no gaps, and the app-level list and
write commands keep working. (Unix only — it needs a real SIGKILL.)

What WAL does **not** cover: a damaged disk, a file synced by a cloud drive from
two machines at once, or an external tool writing to the file. Those need a
restore.

## Restore paths

Ordered from least to most disruptive.

1. **Portable archive import** (in-app, Settings → Backup & Data). Verified
   checksum and referential integrity, full replace, and it takes a
   `pre-import` safety copy first. This is the only path that restores
   **attachment files** along with the records, so prefer it when attachments
   matter.
2. **`restore_database` command** (local API / agent interface). Verifies the
   backup file read-only first (`PRAGMA integrity_check`, a `schema_migrations`
   table, and a version this build supports), takes a `pre-restore` safety copy,
   swaps the file, then reopens and migrates forward. Records only — attachment
   files on disk are untouched, so an attachment row restored from an older
   backup may report `exists: false` from `attachment_path`.
3. **Copying a `.bak` by hand.** With the app closed: move
   `contractorcrm.sqlite3`, `-wal`, and `-shm` somewhere safe (do not delete
   them), copy the chosen `.bak` into place as `contractorcrm.sqlite3`, and start
   the app. It migrates forward on open. Keep the originals until you have
   confirmed the restored data is right.

Every backup and export refuses to write onto the live database file or its
sidecars, even with overwrite enabled.

## Damaged database: symptoms and steps

Symptoms:

- The app fails to start, or reports `stored data is invalid: the database file
  at <path> could not be read and looks damaged …`.
- SQLite reports `file is not a database` or `database disk image is malformed`.
- The database file's size dropped to zero or is wildly smaller than it was. A
  zero-byte file is reported as `stored data is invalid: the database file at
  <path> is empty (zero bytes) …` — SQLite would read it as a valid *empty*
  database, so the app refuses it rather than migrating a fresh schema in and
  showing you an empty CRM.

A damaged file is detected at open — SQLite opens lazily, so the first pragma is
where it surfaces — and returned as `ApplicationError::InvalidStoredData`
(kind `invalid_stored_data`) with a message naming the file and pointing here,
rather than a bare SQLite code. The same mapping applies to the read-only agent
helper (`Storage::open_existing`), and `verify_backup_file` refuses the file as a
restore source (kind `restore_invalid`). Nothing is deleted or overwritten
automatically.

Steps:

1. Close the app. Do not delete anything yet — the `-wal` file may hold the most
   recent writes.
2. Copy the whole application-data directory somewhere else. Work on the copy.
3. List the `.bak` files and pick the newest one that predates the trouble.
4. Restore it (path 1, 2, or 3 above).
5. If every `.bak` is also damaged, fall back to the newest portable archive.
6. Re-check attachments after restoring from a `.bak`: the rows come back with
   the database, the files do not.

Useful support data to keep from the original copy: the file sizes and
timestamps of `contractorcrm.sqlite3` and its sidecars, the exact error text, the
list of `.bak` file names, and the output of `PRAGMA integrity_check` on the
damaged file (`sqlite3 contractorcrm.sqlite3 "PRAGMA integrity_check;"`).
