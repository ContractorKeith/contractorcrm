//! Crash recovery: kill a writing process with SIGKILL and prove the database
//! comes back consistent.
//!
//! The writer is this same test binary re-invoked with `CRASH_CHILD_DATABASE`
//! set (the `crash_writer_child` test below, `#[ignore]`d so a normal run never
//! executes it). It writes contacts through the application seam in a tight
//! loop and prints each committed sequence number; the parent kills it mid-run
//! and reopens the database through the normal `Storage::open` path.
//!
//! Unix only: it needs a real SIGKILL (`Child::kill`). On Windows CI this file
//! compiles to nothing — the equivalent there is `TerminateProcess`, which
//! would need its own harness.

#![cfg(unix)]

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};

use contractorcrm_lib::application::{
    self, ChannelInput, ContactPatch, CreateContactRequest, UpdateContactRequest,
};
use contractorcrm_lib::storage::Storage;

/// Set on the re-invoked child; its value is the database path to hammer.
const CHILD_DATABASE_ENV: &str = "CRASH_CHILD_DATABASE";

/// How many committed writes the parent waits for before pulling the plug.
const COMMITS_BEFORE_KILL: usize = 8;

/// How many kill/reopen cycles one test run performs against the same file.
const KILL_ITERATIONS: usize = 5;

/// One create-contact transaction: contact row + channel + search projection +
/// command-log entry, all or nothing. `sequence` is embedded in every one of
/// them so the parent can check they land together.
fn write_marked_contact(storage: &mut Storage, sequence: u64) {
    application::create_contact(
        storage,
        CreateContactRequest {
            actor: Default::default(),
            contact: ContactPatch {
                display_name: Some(format!("Crash {sequence:06}")),
                first_name: Some("Crash".into()),
                last_name: Some(format!("{sequence:06}")),
                kind: "client".into(),
                notes: Some(format!("crash-marker-{sequence:06}")),
                channels: vec![ChannelInput {
                    kind: "phone".into(),
                    label: Some("mobile".into()),
                    value: format!("555-{sequence:06}"),
                    preferred: true,
                }],
                ..Default::default()
            },
        },
    )
    .expect("create contact");
}

/// The writer process. Picks up where the last (killed) run left off so a file
/// can be crashed repeatedly and still carry one contiguous sequence.
#[test]
#[ignore = "child process of crash_recovery tests; started with CRASH_CHILD_DATABASE"]
fn crash_writer_child() {
    let Ok(database_path) = std::env::var(CHILD_DATABASE_ENV) else {
        return; // Never started directly.
    };
    let mut storage = Storage::open(&database_path).expect("child opens storage");
    let existing: i64 = storage
        .connection()
        .query_row("SELECT COUNT(*) FROM contacts", [], |row| row.get(0))
        .expect("count existing contacts");
    let mut sequence = existing as u64;

    loop {
        write_marked_contact(&mut storage, sequence);
        // Printed only after commit — the parent treats each line as durable.
        println!("committed {sequence}");
        sequence += 1;
    }
}

/// Start the writer and block until it has committed `COMMITS_BEFORE_KILL`
/// records, then SIGKILL it. Returns the highest sequence known committed.
fn crash_a_writer(database_path: &Path) -> u64 {
    let mut child = Command::new(std::env::current_exe().expect("test binary path"))
        .args(["--exact", "crash_writer_child", "--ignored", "--nocapture"])
        .env(CHILD_DATABASE_ENV, database_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn crash writer");

    let stdout = child.stdout.take().expect("child stdout");
    let mut reader = BufReader::new(stdout);
    let mut committed = 0usize;
    let mut highest = 0u64;
    let mut line = String::new();
    while committed < COMMITS_BEFORE_KILL {
        line.clear();
        let read = reader.read_line(&mut line).expect("read child output");
        assert!(read > 0, "crash writer exited before writing anything");
        if let Some(sequence) = line.trim().strip_prefix("committed ") {
            highest = sequence.parse().expect("sequence number");
            committed += 1;
        }
    }

    // SIGKILL on unix: no unwinding, no flush, no clean SQLite shutdown. The
    // child is mid-transaction on the next record almost every time.
    child.kill().expect("kill crash writer");
    let status = child.wait().expect("reap crash writer");
    assert!(!status.success(), "writer should die by signal, not exit 0");
    // A clean close removes the WAL sidecar; finding it proves this was not one.
    let mut wal = database_path.as_os_str().to_os_string();
    wal.push("-wal");
    assert!(
        Path::new(&wal).is_file(),
        "no WAL left behind — the writer was not killed mid-flight"
    );
    highest
}

/// Reopen through the normal path and assert the database is intact: SQLite
/// integrity, no half-written record, contiguous sequence, working commands.
fn assert_consistent_after_crash(database_path: &Path, at_least: u64) {
    let storage = Storage::open(database_path).expect("reopen after crash");

    let integrity: String = storage
        .connection()
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .expect("integrity check runs");
    assert_eq!(integrity, "ok", "database is corrupt after SIGKILL");

    let foreign_key_violations: i64 = storage
        .connection()
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .expect("foreign key check runs");
    assert_eq!(foreign_key_violations, 0);

    // Every committed contact must carry the whole transaction: its channel,
    // its search projection, and its command-log entry. A torn transaction
    // shows up here as a contact missing one of the three.
    let orphans: i64 = storage
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM contacts c WHERE
               (SELECT COUNT(*) FROM contact_channels ch WHERE ch.contact_id = c.id) <> 1
               OR NOT EXISTS (SELECT 1 FROM search_index s
                              WHERE s.entity_type = 'contact' AND s.entity_id = c.id)
               OR NOT EXISTS (SELECT 1 FROM command_log l
                              WHERE l.entity_type = 'contact' AND l.entity_id = c.id)",
            [],
            |row| row.get(0),
        )
        .expect("partial record check runs");
    assert_eq!(orphans, 0, "a record was only partially written");

    // The writer is sequential, so the surviving names must be 0..count with no
    // gaps, and must include everything the child reported as committed.
    let contacts = application::list_contacts(&storage, true).expect("list contacts still works");
    let mut sequences: Vec<u64> = contacts
        .iter()
        .map(|item| {
            item.contact
                .display_name
                .strip_prefix("Crash ")
                .expect("crash marker name")
                .parse()
                .expect("sequence number")
        })
        .collect();
    sequences.sort_unstable();
    let expected: Vec<u64> = (0..sequences.len() as u64).collect();
    assert_eq!(sequences, expected, "committed sequence has a gap");
    assert!(
        sequences.len() as u64 > at_least,
        "records reported committed are missing: have {}, need more than {at_least}",
        sequences.len()
    );

    // Every channel value matches its contact's sequence — proof the two halves
    // of a transaction never come from different transactions.
    let mismatched: i64 = storage
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM contacts c JOIN contact_channels ch ON ch.contact_id = c.id
             WHERE ch.value <> '555-' || c.last_name
                OR c.notes <> 'crash-marker-' || c.last_name",
            [],
            |row| row.get(0),
        )
        .expect("channel pairing check runs");
    assert_eq!(mismatched, 0, "a transaction's rows disagree");
}

#[test]
fn sigkill_during_writes_leaves_a_consistent_database() {
    let temp = tempfile::tempdir().expect("tempdir");
    let database_path = temp.path().join("contractorcrm.sqlite3");

    for iteration in 0..KILL_ITERATIONS {
        let highest = crash_a_writer(&database_path);
        assert_consistent_after_crash(&database_path, highest);
        // Sidecars are expected after a kill; the reopen above recovered them.
        assert!(
            database_path.is_file(),
            "database file vanished on iteration {iteration}"
        );
    }
}

#[test]
fn writes_still_work_after_a_crash_and_recovery() {
    let temp = tempfile::tempdir().expect("tempdir");
    let database_path = temp.path().join("contractorcrm.sqlite3");
    let highest = crash_a_writer(&database_path);
    assert_consistent_after_crash(&database_path, highest);

    // The recovered database is fully usable, not just readable.
    let mut storage = Storage::open(&database_path).expect("reopen after crash");
    let before = application::list_contacts(&storage, true).expect("list contacts");
    let existing = before
        .first()
        .expect("crash writer left records")
        .contact
        .clone();
    application::update_contact(
        &mut storage,
        UpdateContactRequest {
            actor: Default::default(),
            contact_id: existing.id.clone(),
            expected_version: existing.version,
            patch: ContactPatch {
                display_name: Some(existing.display_name.clone()),
                kind: "client".into(),
                notes: Some("recovered and edited".into()),
                ..Default::default()
            },
        },
    )
    .expect("update a recovered record");

    write_marked_contact(&mut storage, before.len() as u64);
    let after = application::list_contacts(&storage, true).expect("list contacts");
    assert_eq!(after.len(), before.len() + 1);
}
