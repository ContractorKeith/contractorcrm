use std::path::{Path, PathBuf};

use contractorcrm_lib::application as app;
use contractorcrm_lib::attachments::{
    add_attachment, attachment_path, list_attachments, remove_attachment, sanitized_file_name,
    AttachmentParentType, AttachmentStore, MAX_ATTACHMENT_BYTES,
};
use contractorcrm_lib::storage::Storage;
use serde_json::json;

/// A database and its managed attachment root, wired the way production does.
fn setup() -> (tempfile::TempDir, Storage, AttachmentStore) {
    let temp = tempfile::tempdir().unwrap();
    let storage = Storage::open_in_app_data(temp.path().join("data")).unwrap();
    let store = AttachmentStore::open_in_app_data(temp.path().join("data"));
    (temp, storage, store)
}

/// A file outside the managed root, the way a user's file arrives.
fn source_file(temp: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let inbox = temp.join("inbox");
    std::fs::create_dir_all(&inbox).unwrap();
    let path = inbox.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

fn contact(storage: &mut Storage, display_name: &str) -> String {
    app::create_contact(
        storage,
        serde_json::from_value(json!({"displayName": display_name, "kind": "client"})).unwrap(),
    )
    .unwrap()
    .id
}

fn opportunity(storage: &mut Storage, name: &str, contact_id: &str) -> String {
    app::create_opportunity(
        storage,
        serde_json::from_value(
            json!({"name": name, "contactId": contact_id, "currencyCode": "USD"}),
        )
        .unwrap(),
    )
    .unwrap()
    .id
}

fn add(
    storage: &mut Storage,
    store: &AttachmentStore,
    parent_type: &str,
    parent_id: &str,
    source: &Path,
) -> Result<contractorcrm_lib::attachments::Attachment, contractorcrm_lib::error::ApplicationError>
{
    add_attachment(
        storage,
        store,
        serde_json::from_value(json!({
            "parentType": parent_type, "parentId": parent_id,
            "sourcePath": source.to_str().unwrap()
        }))
        .unwrap(),
    )
}

fn command_summaries(storage: &Storage) -> Vec<String> {
    storage
        .connection()
        .prepare("SELECT summary FROM command_log WHERE entity_type = 'attachment' ORDER BY created_at, id")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

#[test]
fn attachments_are_copied_under_management_listed_and_removed() {
    let (temp, mut storage, store) = setup();
    let dana = contact(&mut storage, "Dana Reyes");
    let fence = opportunity(&mut storage, "Backyard fence", &dana);

    let notes = source_file(temp.path(), "site notes.txt", b"gate swings inward");
    let quote = source_file(temp.path(), "cedar quote.pdf", b"%PDF-1.4 cedar");
    let first = add(&mut storage, &store, "contact", &dana, &notes).unwrap();
    let second = add(&mut storage, &store, "opportunity", &fence, &quote).unwrap();

    assert_eq!(first.file_name, "site notes.txt");
    assert_eq!(first.media_type.as_deref(), Some("text/plain"));
    assert_eq!(second.media_type.as_deref(), Some("application/pdf"));
    assert_eq!(first.size_bytes, b"gate swings inward".len() as i64);
    assert_eq!(first.sha256.len(), 64);
    assert_eq!(first.version, 1);
    assert_eq!(first.parent_type, AttachmentParentType::Contact);

    // The managed copy sits at <root>/<id>/<file name>; the original is intact.
    let managed = store.root().join(&first.id).join(&first.file_name);
    assert_eq!(std::fs::read(&managed).unwrap(), b"gate swings inward");
    assert!(notes.exists(), "the user's file is copied, not moved");

    // Listing is per parent and ordered oldest first.
    let listed = list_attachments(&storage, AttachmentParentType::Contact, &dana).unwrap();
    assert_eq!(listed, vec![first.clone()]);
    let opportunity_files =
        list_attachments(&storage, AttachmentParentType::Opportunity, &fence).unwrap();
    assert_eq!(opportunity_files, vec![second.clone()]);

    // A stale version cannot remove the row.
    let conflict = remove_attachment(
        &mut storage,
        &store,
        serde_json::from_value(json!({"attachmentId": first.id, "expectedVersion": 2})).unwrap(),
    )
    .unwrap_err();
    assert_eq!(conflict.kind(), "version_conflict");
    assert!(managed.exists(), "a refused removal keeps the file");

    let removal = remove_attachment(
        &mut storage,
        &store,
        serde_json::from_value(json!({"attachmentId": first.id, "expectedVersion": 1})).unwrap(),
    )
    .unwrap();
    assert!(removal.file_removed);
    assert!(!store.root().join(&first.id).exists());
    assert!(
        list_attachments(&storage, AttachmentParentType::Contact, &dana)
            .unwrap()
            .is_empty()
    );

    // Every write is in the audit log; the opportunity's file is untouched.
    let summaries = command_summaries(&storage);
    assert_eq!(summaries.len(), 3, "{summaries:?}");
    assert!(summaries[0].contains("attached \"site notes.txt\""));
    assert!(summaries[2].starts_with("removed attachment \"site notes.txt\""));
    assert!(store.root().join(&second.id).exists());
}

#[test]
fn removing_an_unknown_attachment_is_not_found() {
    let (_temp, mut storage, store) = setup();
    let error = remove_attachment(
        &mut storage,
        &store,
        serde_json::from_value(json!({"attachmentId": "nope", "expectedVersion": 1})).unwrap(),
    )
    .unwrap_err();
    assert_eq!(error.kind(), "not_found");
    assert_eq!(
        attachment_path(&storage, &store, "nope")
            .unwrap_err()
            .kind(),
        "not_found"
    );
}

#[test]
fn file_names_are_sanitized_into_a_safe_managed_layout() {
    // Separators and traversal collapse to the base name.
    assert_eq!(
        sanitized_file_name("../../etc/passwd").unwrap(),
        "passwd",
        "traversal cannot escape the store"
    );
    assert_eq!(
        sanitized_file_name("C:\\Windows\\system32\\drivers.txt").unwrap(),
        "drivers.txt"
    );
    // Control characters and trailing dots or spaces are stripped.
    assert_eq!(
        sanitized_file_name("re\u{7}port.pdf ").unwrap(),
        "report.pdf"
    );
    assert_eq!(sanitized_file_name("quote.pdf...").unwrap(), "quote.pdf");
    // Nothing usable left is a caller error, not an empty path.
    for raw in ["", "   ", "...", "/", "\u{7}"] {
        assert_eq!(
            sanitized_file_name(raw).unwrap_err().kind(),
            "invalid_input",
            "{raw:?}"
        );
    }
    // Windows device names are never used bare.
    for raw in ["CON", "nul.txt", "COM9.pdf", "LPT1"] {
        assert!(
            sanitized_file_name(raw).unwrap().starts_with('_'),
            "{raw} must be guarded"
        );
    }
    // Invisible format characters are stripped: a right-to-left override can
    // make an executable render as a PDF.
    assert_eq!(
        sanitized_file_name("photo\u{202e}fdp.exe").unwrap(),
        "photofdp.exe"
    );
    assert_eq!(
        sanitized_file_name("in\u{200b}voice\u{feff}.pdf").unwrap(),
        "invoice.pdf"
    );
    assert_eq!(
        sanitized_file_name("\u{202e}\u{200b}").unwrap_err().kind(),
        "invalid_input"
    );
    assert_eq!(sanitized_file_name("contract.pdf").unwrap(), "contract.pdf");
    // Long names are capped in bytes but keep their extension.
    let long = format!("{}.pdf", "a".repeat(400));
    let capped = sanitized_file_name(&long).unwrap();
    assert!(capped.len() <= 120, "{}", capped.len());
    assert!(capped.ends_with(".pdf"));
    // Unicode survives, and truncation lands on a character boundary.
    assert_eq!(
        sanitized_file_name("契約書 – 見積.pdf").unwrap(),
        "契約書 – 見積.pdf"
    );
    let long_unicode = format!("{}.pdf", "契約書".repeat(100));
    let capped_unicode = sanitized_file_name(&long_unicode).unwrap();
    assert!(capped_unicode.len() <= 120, "{}", capped_unicode.len());
    assert!(capped_unicode.ends_with(".pdf"));
}

#[test]
fn a_sanitized_name_is_what_lands_on_disk() {
    let (temp, mut storage, store) = setup();
    let dana = contact(&mut storage, "Dana Reyes");
    let source = source_file(temp.path(), "CON.txt", b"device name");
    let attachment = add(&mut storage, &store, "contact", &dana, &source).unwrap();

    assert_eq!(attachment.file_name, "_CON.txt");
    assert!(store.root().join(&attachment.id).join("_CON.txt").is_file());
    // A spoofed name lands on disk without its override character.
    let spoofed = source_file(temp.path(), "photo\u{202e}fdp.exe", b"MZ");
    let attachment = add(&mut storage, &store, "contact", &dana, &spoofed).unwrap();
    assert_eq!(attachment.file_name, "photofdp.exe");
    assert!(store
        .root()
        .join(&attachment.id)
        .join("photofdp.exe")
        .is_file());

    let unicode = source_file(temp.path(), "見積 2026.pdf", b"%PDF-1.4");
    let attachment = add(&mut storage, &store, "contact", &dana, &unicode).unwrap();
    assert_eq!(attachment.file_name, "見積 2026.pdf");
    assert_eq!(attachment.media_type.as_deref(), Some("application/pdf"));
}

#[test]
fn a_file_past_the_size_cap_is_refused_before_it_is_copied() {
    let (temp, mut storage, store) = setup();
    let dana = contact(&mut storage, "Dana Reyes");
    let inbox = temp.path().join("inbox");
    std::fs::create_dir_all(&inbox).unwrap();
    // Sparse file: the cap is checked from the metadata, so nothing is written.
    let source = inbox.join("huge.zip");
    let file = std::fs::File::create(&source).unwrap();
    file.set_len(MAX_ATTACHMENT_BYTES + 1).unwrap();
    drop(file);

    let error = add(&mut storage, &store, "contact", &dana, &source).unwrap_err();
    assert_eq!(error.kind(), "validation_failed");
    assert!(error.to_string().contains("limited to"), "{error}");
    assert!(
        list_attachments(&storage, AttachmentParentType::Contact, &dana)
            .unwrap()
            .is_empty()
    );
    assert!(
        !store.root().exists() || std::fs::read_dir(store.root()).unwrap().next().is_none(),
        "a refused add leaves no managed file"
    );
}

#[test]
fn attaching_needs_an_existing_active_parent_and_a_readable_file() {
    let (temp, mut storage, store) = setup();
    let dana = contact(&mut storage, "Dana Reyes");
    let fence = opportunity(&mut storage, "Backyard fence", &dana);
    let source = source_file(temp.path(), "notes.txt", b"notes");

    // Missing parent.
    let error = add(&mut storage, &store, "contact", "contact-nope", &source).unwrap_err();
    assert_eq!(error.kind(), "invalid_input");
    // Archived parent.
    let contact_record = app::get_contact(&storage, &dana).unwrap();
    app::archive_contact(
        &mut storage,
        serde_json::from_value(json!({"id": dana, "expectedVersion": contact_record.version}))
            .unwrap(),
    )
    .unwrap();
    let error = add(&mut storage, &store, "contact", &dana, &source).unwrap_err();
    assert_eq!(error.kind(), "invalid_input");
    assert!(error.to_string().contains("archived"), "{error}");

    // Unreadable and non-file sources.
    let missing = temp.path().join("inbox").join("gone.txt");
    let error = add(&mut storage, &store, "opportunity", &fence, &missing).unwrap_err();
    assert_eq!(error.kind(), "invalid_input");
    let error = add(
        &mut storage,
        &store,
        "opportunity",
        &fence,
        &temp.path().join("inbox"),
    )
    .unwrap_err();
    assert_eq!(error.kind(), "invalid_input");
}

#[test]
fn a_managed_file_cannot_be_attached_to_the_store_again() {
    let (temp, mut storage, store) = setup();
    let dana = contact(&mut storage, "Dana Reyes");
    let source = source_file(temp.path(), "notes.txt", b"notes");
    let attachment = add(&mut storage, &store, "contact", &dana, &source).unwrap();

    let managed = store
        .root()
        .join(&attachment.id)
        .join(&attachment.file_name);
    let error = add(&mut storage, &store, "contact", &dana, &managed).unwrap_err();
    assert_eq!(error.kind(), "invalid_input");
    assert!(error.to_string().contains("already a managed"), "{error}");
    assert_eq!(
        list_attachments(&storage, AttachmentParentType::Contact, &dana)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn attachment_path_is_absolute_and_reports_a_missing_file() {
    let (temp, mut storage, store) = setup();
    let dana = contact(&mut storage, "Dana Reyes");
    let source = source_file(temp.path(), "notes.txt", b"notes");
    let attachment = add(&mut storage, &store, "contact", &dana, &source).unwrap();

    let location = attachment_path(&storage, &store, &attachment.id).unwrap();
    assert!(Path::new(&location.path).is_absolute());
    assert!(location.path.ends_with("notes.txt"));
    assert!(location.exists);

    // A file the user deleted behind our back is reported, not an error.
    std::fs::remove_file(&location.path).unwrap();
    let location = attachment_path(&storage, &store, &attachment.id).unwrap();
    assert!(!location.exists);

    // Removing the row still succeeds and reports the cleanup.
    let removal = remove_attachment(
        &mut storage,
        &store,
        serde_json::from_value(
            json!({"attachmentId": attachment.id, "expectedVersion": attachment.version}),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(removal.file_removed);
    assert!(!store.root().join(&attachment.id).exists());
}

#[test]
fn migration_010_creates_attachments_on_fresh_and_upgraded_databases() {
    let temp = tempfile::tempdir().unwrap();
    let database_path = temp.path().join("contractorcrm.sqlite3");
    let storage = Storage::open(&database_path).unwrap();
    // Attachments arrived in the tenth migration; later ones may follow it.
    assert!(
        contractorcrm_lib::storage::latest_migration_version() >= 10,
        "attachments is the tenth migration"
    );
    let columns: Vec<String> = storage
        .connection()
        .prepare("SELECT name FROM pragma_table_info('attachments') ORDER BY cid")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(
        columns,
        [
            "id",
            "parent_type",
            "parent_id",
            "file_name",
            "relative_path",
            "media_type",
            "size_bytes",
            "sha256",
            "created_at",
            "version"
        ]
    );

    // Roll the schema back to version 9 and reopen: the migration reruns.
    storage
        .connection()
        .execute_batch(
            "DROP TRIGGER contacts_attachments_delete;
             DROP TRIGGER opportunities_attachments_delete;
             DROP TABLE attachments;
             DELETE FROM schema_migrations WHERE version = 10;",
        )
        .unwrap();
    drop(storage);

    let storage = Storage::open(&database_path).unwrap();
    let applied: Vec<i64> = storage
        .connection()
        .prepare("SELECT version FROM schema_migrations ORDER BY version")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(
        applied,
        (1..=contractorcrm_lib::storage::latest_migration_version()).collect::<Vec<_>>()
    );
    let table_exists: bool = storage
        .connection()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='attachments')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(table_exists);
    // Upgrading an existing database leaves a pre-migration safety copy.
    assert!(temp
        .path()
        .join("contractorcrm.sqlite3.pre-migration-v10.bak")
        .is_file());
}

#[test]
fn the_parent_trigger_refuses_rows_without_a_live_parent() {
    let (_temp, storage, _store) = setup();
    let error = storage
        .connection()
        .execute(
            "INSERT INTO attachments (id, parent_type, parent_id, file_name, relative_path,
                                      media_type, size_bytes, sha256, created_at, version)
             VALUES ('a1', 'contact', 'ghost', 'x.txt', 'a1/x.txt', 'text/plain', 1, 'abc',
                     '2026-08-18T00:00:00.000Z', 1)",
            [],
        )
        .unwrap_err();
    assert!(
        error.to_string().contains("missing attachment parent"),
        "{error}"
    );
}

#[test]
fn a_drive_relative_stored_path_never_leaves_the_managed_root() {
    // On Windows a path component like "C:evil" carries a drive prefix without
    // a root, so pushing it onto the managed root replaces the whole path and
    // addresses the current directory of drive C. Every consumer of a stored
    // path refuses it instead, on every platform, so an archive written on one
    // machine cannot plant an escape that only fires on another.
    let (temp, mut storage, store) = setup();
    let contact_id = contact(&mut storage, "Dana Ruiz");
    let source = source_file(temp.path(), "scope.txt", b"managed bytes");
    let attachment = add(&mut storage, &store, "contact", &contact_id, &source).unwrap();

    for poisoned in ["C:evil/scope.txt", "C:/scope.txt", "sub/C:evil.txt"] {
        storage
            .connection()
            .execute(
                "UPDATE attachments SET relative_path = ?1 WHERE id = ?2",
                rusqlite::params![poisoned, attachment.id],
            )
            .unwrap();
        let error = attachment_path(&storage, &store, &attachment.id).unwrap_err();
        assert_eq!(error.kind(), "invalid_stored_data", "{poisoned}");
    }

    // The same rule bounds the file name the user's own file can produce.
    assert_eq!(sanitized_file_name("C:evil.txt").unwrap(), "Cevil.txt");
}
