//! Backup/restore integration tests — round-trip fidelity, overwrite guard,
//! corrupted/newer-schema rejection, forward migration, and safety copies.

use std::path::Path;

use contractorcrm_lib::application::{
    self, ChannelInput, CompanyPatch, ContactPatch, CreateCompanyRequest, CreateContactRequest,
};
use contractorcrm_lib::domain::{Company, Contact};
use contractorcrm_lib::storage::Storage;

/// Seed one company and one contact (with channels) through the application seam.
fn seed_records(storage: &mut Storage) -> (Company, Contact) {
    let company = application::create_company(
        storage,
        CreateCompanyRequest {
            actor: Default::default(),
            company: CompanyPatch {
                name: "Ridgeline Fence Co".into(),
                kind: "sub".into(),
                phone: Some("555-0100".into()),
                ..Default::default()
            },
        },
    )
    .expect("create company");
    let contact = application::create_contact(
        storage,
        CreateContactRequest {
            actor: Default::default(),
            contact: ContactPatch {
                company_id: Some(company.id.clone()),
                first_name: Some("Dana".into()),
                last_name: Some("Ruiz".into()),
                kind: "client".into(),
                channels: vec![ChannelInput {
                    kind: "phone".into(),
                    label: Some("mobile".into()),
                    value: "555-0101".into(),
                    preferred: true,
                }],
                ..Default::default()
            },
        },
    )
    .expect("create contact");
    (company, contact)
}

/// All pre-restore safety copies sitting next to a database file.
fn safety_copies(database_path: &Path) -> Vec<std::path::PathBuf> {
    let directory = database_path.parent().expect("database directory");
    std::fs::read_dir(directory)
        .expect("read database directory")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains(".pre-restore-"))
        })
        .collect()
}

#[test]
fn backup_restore_round_trip_preserves_records_and_versions() {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut source = Storage::open(temp.path().join("source.sqlite3")).expect("open source");
    let (company, contact) = seed_records(&mut source);
    // Bump the contact so a non-default version must survive the round trip.
    let contact = application::archive_contact(
        &mut source,
        application::ArchiveRequest {
            actor: Default::default(),
            id: contact.id.clone(),
            expected_version: contact.version,
        },
    )
    .expect("archive contact");
    assert_eq!(contact.version, 2);

    let backup_path = temp.path().join("backups/nested/crm-backup.sqlite3");
    application::backup_database(&mut source, backup_path.to_str().unwrap(), false)
        .expect("backup with nested parent dirs");

    // Fresh target database with its own throwaway record, then restore.
    let mut target = Storage::open(temp.path().join("target.sqlite3")).expect("open target");
    seed_records(&mut target);
    let report = application::restore_database(&mut target, backup_path.to_str().unwrap())
        .expect("restore into target");
    assert!(Path::new(&report.safety_copy_path).is_file());

    let source_companies = application::list_companies(&source, true).expect("source companies");
    let target_companies = application::list_companies(&target, true).expect("target companies");
    assert_eq!(source_companies, target_companies);
    assert_eq!(target_companies, vec![company]);

    let source_contacts = application::list_contacts(&source, true).expect("source contacts");
    let target_contacts = application::list_contacts(&target, true).expect("target contacts");
    assert_eq!(source_contacts, target_contacts);
    assert_eq!(target_contacts, vec![contact]);
    assert_eq!(target_contacts[0].version, 2);
}

#[test]
fn backup_records_last_backup_timestamp_in_database_info() {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut storage = Storage::open(temp.path().join("crm.sqlite3")).expect("open");

    let before = application::get_database_info(&storage).expect("info before backup");
    assert_eq!(before.last_backup_at, None);
    assert!(before.file_size_bytes > 0);

    let backup_path = temp.path().join("backup.sqlite3");
    let after = application::backup_database(&mut storage, backup_path.to_str().unwrap(), false)
        .expect("backup");
    assert!(after.last_backup_at.is_some());
    assert_eq!(after.database_path, before.database_path);
}

#[test]
fn backup_refuses_to_overwrite_without_the_flag() {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut storage = Storage::open(temp.path().join("crm.sqlite3")).expect("open");
    let backup_path = temp.path().join("backup.sqlite3");
    let backup_path = backup_path.to_str().unwrap();

    application::backup_database(&mut storage, backup_path, false).expect("first backup");
    let error = application::backup_database(&mut storage, backup_path, false)
        .expect_err("second backup without overwrite must fail");
    assert_eq!(error.kind(), "backup_failed");

    // Explicit overwrite succeeds.
    application::backup_database(&mut storage, backup_path, true).expect("overwrite backup");
}

#[test]
fn restore_rejects_a_corrupted_file_and_leaves_the_live_database_untouched() {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut storage = Storage::open(temp.path().join("crm.sqlite3")).expect("open");
    let (company, _) = seed_records(&mut storage);

    let garbage_path = temp.path().join("garbage.sqlite3");
    std::fs::write(&garbage_path, b"this is not a sqlite database at all").expect("write garbage");

    let error = application::restore_database(&mut storage, garbage_path.to_str().unwrap())
        .expect_err("garbage file must be rejected");
    assert_eq!(error.kind(), "restore_invalid");

    // Live database still serves the seeded records; no safety copy was made.
    let companies = application::list_companies(&storage, true).expect("list after failed restore");
    assert_eq!(companies, vec![company]);
    assert!(safety_copies(storage.database_path()).is_empty());
}

#[test]
fn restore_rejects_a_backup_with_a_newer_schema_version() {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut storage = Storage::open(temp.path().join("crm.sqlite3")).expect("open");
    seed_records(&mut storage);

    // A structurally valid database claiming a schema from the future.
    let future_path = temp.path().join("future.sqlite3");
    let connection = rusqlite::Connection::open(&future_path).expect("create future backup");
    connection
        .execute_batch(
            "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);
             INSERT INTO schema_migrations (version, applied_at) VALUES (999, '2999-01-01T00:00:00Z');",
        )
        .expect("seed future migration record");
    drop(connection);

    let error = application::restore_database(&mut storage, future_path.to_str().unwrap())
        .expect_err("newer schema must be rejected");
    assert_eq!(error.kind(), "restore_invalid");
    assert!(safety_copies(storage.database_path()).is_empty());

    // A database with no schema_migrations table at all is also rejected.
    let foreign_path = temp.path().join("foreign.sqlite3");
    let connection = rusqlite::Connection::open(&foreign_path).expect("create foreign db");
    connection
        .execute_batch("CREATE TABLE other (id INTEGER PRIMARY KEY);")
        .expect("seed foreign table");
    drop(connection);
    let error = application::restore_database(&mut storage, foreign_path.to_str().unwrap())
        .expect_err("non-CRM database must be rejected");
    assert_eq!(error.kind(), "restore_invalid");
}

#[test]
fn restoring_an_older_schema_backup_migrates_forward() {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut storage = Storage::open(temp.path().join("crm.sqlite3")).expect("open");
    seed_records(&mut storage);

    // Simulate a pre-v1 backup: schema_migrations exists but records nothing,
    // so every migration is still pending after the restore.
    let old_path = temp.path().join("old-backup.sqlite3");
    let connection = rusqlite::Connection::open(&old_path).expect("create old backup");
    connection
        .execute_batch(
            "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);",
        )
        .expect("seed empty migration ledger");
    drop(connection);

    application::restore_database(&mut storage, old_path.to_str().unwrap())
        .expect("restore older backup");

    // Migration 1 ran forward on reopen: the v1 tables exist and are usable.
    let applied: i64 = storage
        .connection()
        .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .expect("read applied version");
    assert_eq!(applied, 1);
    let companies = application::list_companies(&storage, true).expect("list on migrated restore");
    assert!(companies.is_empty());
}

#[test]
fn restore_keeps_a_timestamped_safety_copy_of_the_previous_database() {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut storage = Storage::open(temp.path().join("crm.sqlite3")).expect("open");
    let (company, _) = seed_records(&mut storage);

    let backup_path = temp.path().join("backup.sqlite3");
    application::backup_database(&mut storage, backup_path.to_str().unwrap(), false)
        .expect("backup");
    let report = application::restore_database(&mut storage, backup_path.to_str().unwrap())
        .expect("restore");

    let copies = safety_copies(storage.database_path());
    assert_eq!(
        copies,
        vec![std::path::PathBuf::from(&report.safety_copy_path)]
    );

    // The safety copy is itself a valid database holding the old records.
    let safety = Storage::open(&copies[0]).expect("open safety copy");
    let companies = application::list_companies(&safety, true).expect("list safety copy");
    assert_eq!(companies, vec![company]);
}
