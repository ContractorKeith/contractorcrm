use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use contractorcrm_lib::archive::{export_archive, import_archive, preview_archive_import};
use contractorcrm_lib::attachments::AttachmentStore;
use contractorcrm_lib::storage::Storage;
use rusqlite::types::Value;
use serde_json::json;
use sha2::{Digest, Sha256};

/// Every table an archive carries, in the order the exporter writes them.
const ARCHIVE_TABLES: &[&str] = &[
    "companies",
    "contacts",
    "contact_channels",
    "pipelines",
    "stages",
    "lost_reasons",
    "opportunities",
    "stage_history",
    "activities",
    "tasks",
    "saved_views",
    "tags",
    "record_tags",
    "custom_field_defs",
    "custom_field_options",
    "custom_field_values",
    "attachments",
];

fn storage(temp: &Path, name: &str) -> Storage {
    Storage::open_in_app_data(temp.join(name)).unwrap()
}

/// Managed attachment root beside the database, the way production wires it.
fn attachments(temp: &Path, name: &str) -> AttachmentStore {
    AttachmentStore::open_in_app_data(temp.join(name))
}

/// Write a source file outside the store and attach it to a record.
fn attach(
    storage: &mut Storage,
    store: &AttachmentStore,
    source_dir: &Path,
    parent_type: &str,
    parent_id: &str,
    file_name: &str,
    bytes: &[u8],
) -> contractorcrm_lib::attachments::Attachment {
    std::fs::create_dir_all(source_dir).unwrap();
    let source = source_dir.join(file_name);
    std::fs::write(&source, bytes).unwrap();
    contractorcrm_lib::attachments::add_attachment(
        storage,
        store,
        serde_json::from_value(json!({
            "parentType": parent_type, "parentId": parent_id,
            "sourcePath": source.to_str().unwrap()
        }))
        .unwrap(),
    )
    .unwrap()
}

/// A database with rows in every canonical table: archived and active records,
/// channels, stage history, activities on all three parents, personal and
/// parented tasks, saved views, tags, and all four custom field types.
fn populated(storage: &mut Storage, store: &AttachmentStore, source_dir: &Path) {
    use contractorcrm_lib::application as app;

    let acme: contractorcrm_lib::domain::Company = app::create_company(
        storage,
        serde_json::from_value(json!({"name": "Acme Fence", "kind": "client",
            "phone": "555-0100", "notes": "Repeat customer"}))
        .unwrap(),
    )
    .unwrap();
    let retired = app::create_company(
        storage,
        serde_json::from_value(json!({"name": "Retired Supply", "kind": "vendor"})).unwrap(),
    )
    .unwrap();
    app::archive_company(
        storage,
        serde_json::from_value(json!({"id": retired.id, "expectedVersion": retired.version}))
            .unwrap(),
    )
    .unwrap();

    let dana = app::create_contact(
        storage,
        serde_json::from_value(json!({
            "companyId": acme.id, "firstName": "Dana", "lastName": "Reyes", "kind": "client",
            "favorite": true, "notes": "Prefers texts",
            "channels": [
                {"kind": "email", "value": "dana@acme.test", "preferred": true},
                {"kind": "phone", "value": "555-0101"}
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    let former = app::create_contact(
        storage,
        serde_json::from_value(json!({"displayName": "Former Lead", "kind": "lead"})).unwrap(),
    )
    .unwrap();
    app::archive_contact(
        storage,
        serde_json::from_value(json!({"id": former.id, "expectedVersion": former.version}))
            .unwrap(),
    )
    .unwrap();

    let fence = app::create_opportunity(
        storage,
        serde_json::from_value(json!({
            "name": "Backyard fence", "contactId": dana.id, "companyId": acme.id,
            "valueMinor": 450_000, "currencyCode": "USD", "probabilityPercent": 60,
            "expectedCloseDate": "2026-09-01", "notes": "Cedar privacy fence"
        }))
        .unwrap(),
    )
    .unwrap();
    let gate = app::create_opportunity(
        storage,
        serde_json::from_value(json!({
            "name": "Gate repair", "contactId": dana.id, "valueMinor": 25_000,
            "currencyCode": "USD"
        }))
        .unwrap(),
    )
    .unwrap();
    // Stage history rows, including a lost move with its reason.
    let gate = app::move_opportunity_stage(
        storage,
        serde_json::from_value(json!({
            "opportunityId": gate.id, "toStageId": "stage-lost",
            "lostReasonId": "lost-reason-price", "expectedVersion": gate.version
        }))
        .unwrap(),
    )
    .unwrap();
    let shelved = app::create_opportunity(
        storage,
        serde_json::from_value(json!({"name": "Shelved deck", "companyId": acme.id,
            "currencyCode": "USD"}))
        .unwrap(),
    )
    .unwrap();
    app::archive_opportunity(
        storage,
        serde_json::from_value(json!({"id": shelved.id, "expectedVersion": shelved.version}))
            .unwrap(),
    )
    .unwrap();

    for (parent_type, parent_id, summary) in [
        ("contact", dana.id.clone(), "Called about the fence line"),
        ("company", acme.id.clone(), "Emailed the annual agreement"),
        (
            "opportunity",
            fence.id.clone(),
            "Site visit for measurements",
        ),
    ] {
        app::log_activity(
            storage,
            serde_json::from_value(json!({
                "parentType": parent_type, "parentId": parent_id, "kind": "note",
                "summary": summary, "body": "Details captured on site"
            }))
            .unwrap(),
        )
        .unwrap();
    }

    let task = app::create_task(
        storage,
        serde_json::from_value(json!({
            "title": "Send the cedar quote", "parentType": "opportunity", "parentId": fence.id,
            "dueAt": "2026-08-20T15:00:00.000Z", "priority": "high"
        }))
        .unwrap(),
    )
    .unwrap();
    app::create_task(
        storage,
        serde_json::from_value(json!({"title": "Order post caps"})).unwrap(),
    )
    .unwrap();
    app::complete_task(
        storage,
        serde_json::from_value(json!({"taskId": task.id, "expectedVersion": task.version}))
            .unwrap(),
    )
    .unwrap();

    for entity_type in ["contact", "company", "opportunity"] {
        app::create_saved_view(
            storage,
            serde_json::from_value(json!({
                "name": format!("Active {entity_type}s"), "entityType": entity_type,
                "definition": {"schemaVersion": 2,
                    "filter": {"includeArchived": false, "tagIdsAll": [], "customFields": []},
                    "sort": {"field": if entity_type == "contact" { "displayName" } else { "name" },
                             "direction": "ascending"}}
            }))
            .unwrap(),
        )
        .unwrap();
    }

    let priority = app::create_tag(
        storage,
        serde_json::from_value(json!({"label": "Priority", "colorRole": "attention"})).unwrap(),
    )
    .unwrap();
    let repeat = app::create_tag(
        storage,
        serde_json::from_value(json!({"label": "Repeat"})).unwrap(),
    )
    .unwrap();

    // One custom field definition of each type, with values on live records.
    let mut definitions = BTreeMap::new();
    for (entity_type, label, field_type) in [
        ("contact", "Referred by", "text"),
        ("contact", "Lot size", "number"),
        ("company", "Contract signed", "date"),
        ("opportunity", "Fence type", "select"),
    ] {
        let definition = app::create_custom_field_def(
            storage,
            serde_json::from_value(json!({
                "entityType": entity_type, "label": label, "fieldType": field_type,
                "options": if field_type == "select" {
                    json!([{"label": "Wood"}, {"label": "Vinyl"}])
                } else {
                    json!([])
                }
            }))
            .unwrap(),
        )
        .unwrap();
        definitions.insert(label, definition);
    }

    let dana = app::get_contact(storage, &dana.id).unwrap();
    app::set_record_metadata(
        storage,
        serde_json::from_value(json!({
            "entityType": "contact", "recordId": dana.id, "expectedVersion": dana.version,
            "tagIds": [priority.id, repeat.id],
            "values": [
                {"definitionId": definitions["Referred by"].id, "textValue": "Neighbor",
                 "numberValue": null, "dateValue": null, "optionId": null},
                {"definitionId": definitions["Lot size"].id, "numberValue": 0.75,
                 "textValue": null, "dateValue": null, "optionId": null}
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    let acme = app::get_company(storage, &acme.id).unwrap();
    app::set_record_metadata(
        storage,
        serde_json::from_value(json!({
            "entityType": "company", "recordId": acme.id, "expectedVersion": acme.version,
            "tagIds": [priority.id],
            "values": [{"definitionId": definitions["Contract signed"].id,
                        "dateValue": "2026-01-15", "textValue": null,
                        "numberValue": null, "optionId": null}]
        }))
        .unwrap(),
    )
    .unwrap();
    let fence_detail = app::get_opportunity(storage, &fence.id).unwrap();
    app::set_record_metadata(
        storage,
        serde_json::from_value(json!({
            "entityType": "opportunity", "recordId": fence.id,
            "expectedVersion": fence_detail.opportunity.version,
            "tagIds": [repeat.id],
            "values": [{"definitionId": definitions["Fence type"].id,
                        "optionId": definitions["Fence type"].options[0].id,
                        "textValue": null, "numberValue": null, "dateValue": null}]
        }))
        .unwrap(),
    )
    .unwrap();
    // Managed attachments on both parent kinds.
    attach(
        storage,
        store,
        source_dir,
        "contact",
        &dana.id,
        "site-notes.txt",
        b"measurements and gate swing",
    );
    attach(
        storage,
        store,
        source_dir,
        "opportunity",
        &fence.id,
        "cedar-quote.pdf",
        b"%PDF-1.4 cedar quote",
    );

    // Keep the lost opportunity referenced so stage history has two rows.
    assert_eq!(gate.stage_id, "stage-lost");
}

/// Every canonical row in rowid order — the comparison a lossless round trip
/// has to satisfy.
fn dump(storage: &Storage) -> BTreeMap<String, Vec<Vec<Value>>> {
    let mut dump = BTreeMap::new();
    for table in ARCHIVE_TABLES {
        let mut statement = storage
            .connection()
            .prepare(&format!("SELECT * FROM \"{table}\" ORDER BY rowid"))
            .unwrap();
        let column_count = statement.column_count();
        let rows = statement
            .query_map([], |row| {
                (0..column_count)
                    .map(|index| row.get::<_, Value>(index))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        dump.insert((*table).to_owned(), rows);
    }
    dump
}

fn read_entries(path: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut entries = BTreeMap::new();
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).unwrap();
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_owned();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        entries.insert(name, bytes);
    }
    entries
}

fn write_entries(path: &Path, entries: &BTreeMap<String, Vec<u8>>) {
    let mut writer = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    for (name, bytes) in entries {
        writer.start_file(name.as_str(), options).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap();
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Recompute the manifest checksums and record counts so a tampered archive
/// stays internally consistent — the way a determined attacker would repackage
/// it. Verification has to catch the content, not the bookkeeping.
fn resign(entries: &mut BTreeMap<String, Vec<u8>>) {
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&entries["manifest.json"]).unwrap();
    let files = entries
        .iter()
        .filter(|(name, _)| name.as_str() != "manifest.json")
        .map(|(name, bytes)| {
            json!({"path": name, "sha256": sha256_hex(bytes), "bytes": bytes.len()})
        })
        .collect::<Vec<_>>();
    manifest["files"] = json!(files);
    let mut counts = serde_json::Map::new();
    for table in ARCHIVE_TABLES {
        if let Some(bytes) = entries.get(&format!("data/{table}.json")) {
            let rows: serde_json::Value = serde_json::from_slice(bytes).unwrap();
            counts.insert((*table).to_owned(), json!(rows.as_array().unwrap().len()));
        }
    }
    manifest["recordCounts"] = serde_json::Value::Object(counts);
    entries.insert(
        "manifest.json".into(),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    );
}

/// Rebuild an archive with one data table replaced, keeping the manifest
/// honest, and return the tampered copy's path.
fn repack(
    temp: &Path,
    source: &Path,
    name: &str,
    mutate: impl Fn(&mut BTreeMap<String, Vec<u8>>),
) -> PathBuf {
    let mut entries = read_entries(source);
    mutate(&mut entries);
    resign(&mut entries);
    let path = temp.join(name);
    write_entries(&path, &entries);
    path
}

fn set_table(entries: &mut BTreeMap<String, Vec<u8>>, table: &str, rows: &serde_json::Value) {
    entries.insert(
        format!("data/{table}.json"),
        serde_json::to_vec_pretty(rows).unwrap(),
    );
}

fn table_rows(entries: &BTreeMap<String, Vec<u8>>, table: &str) -> serde_json::Value {
    serde_json::from_slice(&entries[&format!("data/{table}.json")]).unwrap()
}

/// Export a populated database and hand back the archive path plus its temp dir.
fn exported() -> (tempfile::TempDir, PathBuf, Storage) {
    let temp = tempfile::tempdir().unwrap();
    let mut source = storage(temp.path(), "source");
    let store = attachments(temp.path(), "source");
    populated(&mut source, &store, &temp.path().join("inbox"));
    let path = temp.path().join("crm-archive.zip");
    export_archive(&mut source, &store, path.to_str().unwrap(), false).unwrap();
    (temp, path, source)
}

fn issue_codes(preview: &contractorcrm_lib::archive::ArchiveImportPreview) -> Vec<&str> {
    preview
        .issues
        .iter()
        .map(|issue| issue.code.as_str())
        .collect()
}

#[test]
fn archive_round_trips_every_canonical_row_into_a_fresh_database() {
    let (temp, path, source) = exported();
    let expected = dump(&source);
    assert!(
        expected.values().all(|rows| !rows.is_empty()),
        "{expected:?}"
    );

    let mut target = storage(temp.path(), "target");
    let report = import_archive(
        &mut target,
        &attachments(temp.path(), "target"),
        path.to_str().unwrap(),
    )
    .unwrap();

    assert_eq!(dump(&target), expected);
    for table in ARCHIVE_TABLES {
        assert_eq!(
            report.record_counts[*table] as usize,
            expected[*table].len(),
            "count for {table}"
        );
    }
    // The search projection is rebuilt from the imported rows, not copied.
    let hits = contractorcrm_lib::application::search_records(&target, "cedar".into(), None, None)
        .unwrap();
    assert!(
        hits.iter().any(|hit| hit.title == "Backyard fence"),
        "{hits:?}"
    );
    let archived =
        contractorcrm_lib::application::search_records(&target, "Former".into(), None, None)
            .unwrap();
    assert!(archived.is_empty(), "archived records stay out of search");
    // The import is recorded for audit, with the safety backup path.
    let summary: String = target
        .connection()
        .query_row(
            "SELECT summary FROM command_log WHERE entity_type = 'import' AND entity_id = 'archive'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(summary.contains(&report.safety_backup_path), "{summary}");
}

#[test]
fn export_reports_counts_and_refuses_an_existing_destination() {
    let (temp, path, mut source) = exported();
    let entries = read_entries(&path);
    assert!(entries.contains_key("manifest.json"));
    assert!(entries.contains_key("csv/contacts.csv"));
    assert!(entries.contains_key("csv/opportunities.csv"));
    for table in ARCHIVE_TABLES {
        assert!(
            entries.contains_key(&format!("data/{table}.json")),
            "{table}"
        );
    }

    let report = export_archive(
        &mut source,
        &attachments(temp.path(), "source"),
        path.to_str().unwrap(),
        true,
    )
    .unwrap();
    assert_eq!(report.file_count, entries.len());
    assert_eq!(report.record_counts["contacts"], 2);
    assert_eq!(report.record_counts["record_tags"], 4);

    let error = export_archive(
        &mut source,
        &attachments(temp.path(), "source"),
        path.to_str().unwrap(),
        false,
    )
    .unwrap_err();
    assert_eq!(error.kind(), "validation_failed");
    let fresh = temp.path().join("elsewhere/new.zip");
    export_archive(
        &mut source,
        &attachments(temp.path(), "source"),
        fresh.to_str().unwrap(),
        false,
    )
    .unwrap();
    assert!(fresh.exists(), "missing parent directories are created");
}

#[test]
fn a_tampered_file_is_reported_by_preview_and_refused_by_import() {
    let (temp, path, _source) = exported();
    let mut entries = read_entries(&path);
    let contacts = entries.get_mut("data/contacts.json").unwrap();
    let position = contacts.len() / 2;
    contacts[position] ^= 0x20; // flip one bit of one byte
    write_entries(&path, &entries);

    let mut target = storage(temp.path(), "target");
    let before = dump(&target);
    let preview = preview_archive_import(&target, path.to_str().unwrap()).unwrap();
    assert!(
        issue_codes(&preview).contains(&"checksum_mismatch"),
        "{preview:?}"
    );
    assert_eq!(dump(&target), before, "preview never writes");

    let error = import_archive(
        &mut target,
        &attachments(temp.path(), "target"),
        path.to_str().unwrap(),
    )
    .unwrap_err();
    assert_eq!(error.kind(), "validation_failed");
    assert_eq!(dump(&target), before);
}

#[test]
fn entries_that_escape_the_archive_root_are_rejected() {
    let (temp, path, _source) = exported();
    for (name, code) in [
        ("../evil.json", "entry_path_traversal"),
        ("data/../../evil.json", "entry_path_traversal"),
        ("/etc/evil.json", "entry_path_absolute"),
        ("data\\evil.json", "entry_path_backslash"),
        ("data/secrets.json", "unknown_file"),
    ] {
        let mut entries = read_entries(&path);
        entries.insert(name.to_owned(), b"[]".to_vec());
        resign(&mut entries);
        let tampered = temp.path().join("tampered.zip");
        write_entries(&tampered, &entries);

        let target = storage(temp.path(), "target");
        let preview = preview_archive_import(&target, tampered.to_str().unwrap()).unwrap();
        assert!(
            issue_codes(&preview).contains(&code),
            "{name} should report {code}: {preview:?}"
        );
        drop(target);
        let mut target = storage(temp.path(), "target");
        let error = import_archive(
            &mut target,
            &attachments(temp.path(), "target"),
            tampered.to_str().unwrap(),
        )
        .unwrap_err();
        assert_eq!(error.kind(), "validation_failed", "{name}");
        std::fs::remove_file(&tampered).unwrap();
    }
}

#[test]
fn unsupported_schema_and_database_versions_are_rejected() {
    let (temp, path, _source) = exported();
    for (field, value, code) in [
        ("schemaVersion", 2, "unsupported_schema_version"),
        (
            "databaseMigrationVersion",
            999,
            "unsupported_migration_version",
        ),
    ] {
        let mut entries = read_entries(&path);
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&entries["manifest.json"]).unwrap();
        manifest[field] = json!(value);
        entries.insert(
            "manifest.json".into(),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        );
        let tampered = temp.path().join("versioned.zip");
        write_entries(&tampered, &entries);

        let target = storage(temp.path(), "target");
        let preview = preview_archive_import(&target, tampered.to_str().unwrap()).unwrap();
        assert!(issue_codes(&preview).contains(&code), "{preview:?}");
        drop(target);
        let mut target = storage(temp.path(), "target");
        let error = import_archive(
            &mut target,
            &attachments(temp.path(), "target"),
            tampered.to_str().unwrap(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("version"), "{error}");
        std::fs::remove_file(&tampered).unwrap();
    }
}

#[test]
fn a_dangling_reference_is_rejected_before_anything_is_written() {
    let (temp, path, _source) = exported();
    let mut entries = read_entries(&path);
    let mut contacts: serde_json::Value =
        serde_json::from_slice(&entries["data/contacts.json"]).unwrap();
    contacts[0]["companyId"] = json!("company-that-does-not-exist");
    entries.insert(
        "data/contacts.json".into(),
        serde_json::to_vec_pretty(&contacts).unwrap(),
    );
    resign(&mut entries);
    write_entries(&path, &entries);

    let mut target = storage(temp.path(), "target");
    contractorcrm_lib::application::create_company(
        &mut target,
        serde_json::from_value(json!({"name": "Keep me", "kind": "client"})).unwrap(),
    )
    .unwrap();
    let before = dump(&target);

    let preview = preview_archive_import(&target, path.to_str().unwrap()).unwrap();
    assert!(
        issue_codes(&preview).contains(&"missing_reference"),
        "{preview:?}"
    );
    let error = import_archive(
        &mut target,
        &attachments(temp.path(), "target"),
        path.to_str().unwrap(),
    )
    .unwrap_err();
    assert_eq!(error.kind(), "validation_failed");
    assert_eq!(dump(&target), before, "a refused import writes nothing");
}

#[test]
fn malformed_rows_are_reported_with_stable_codes() {
    let (temp, path, _source) = exported();
    for (mutate, code) in [
        (
            Box::new(|rows: &mut serde_json::Value| rows[0]["surpriseColumn"] = json!("x"))
                as Box<dyn Fn(&mut serde_json::Value)>,
            "unknown_column",
        ),
        (
            Box::new(|rows: &mut serde_json::Value| {
                rows[0].as_object_mut().unwrap().remove("displayName");
            }),
            "missing_column",
        ),
        (
            Box::new(|rows: &mut serde_json::Value| rows[0]["version"] = json!(0)),
            "invalid_version",
        ),
        (
            Box::new(|rows: &mut serde_json::Value| rows[0]["id"] = json!("  ")),
            "invalid_id",
        ),
        (
            Box::new(|rows: &mut serde_json::Value| rows[0]["displayName"] = json!(7)),
            "invalid_value",
        ),
        (
            Box::new(|rows: &mut serde_json::Value| {
                let duplicate = rows[0].clone();
                rows.as_array_mut().unwrap().push(duplicate);
            }),
            "duplicate_primary_key",
        ),
    ] {
        let mut entries = read_entries(&path);
        let mut contacts: serde_json::Value =
            serde_json::from_slice(&entries["data/contacts.json"]).unwrap();
        mutate(&mut contacts);
        entries.insert(
            "data/contacts.json".into(),
            serde_json::to_vec_pretty(&contacts).unwrap(),
        );
        resign(&mut entries);
        let tampered = temp.path().join("rows.zip");
        write_entries(&tampered, &entries);

        let target = storage(temp.path(), "target");
        let preview = preview_archive_import(&target, tampered.to_str().unwrap()).unwrap();
        assert!(issue_codes(&preview).contains(&code), "{code}: {preview:?}");
        std::fs::remove_file(&tampered).unwrap();
    }
}

#[test]
fn preview_summarizes_a_clean_archive_without_issues() {
    let (temp, path, source) = exported();
    let target = storage(temp.path(), "target");
    let preview = preview_archive_import(&target, path.to_str().unwrap()).unwrap();

    assert_eq!(preview.issues, vec![]);
    assert_eq!(preview.schema_version, 1);
    assert_eq!(preview.product.name, "ContractorCRM");
    assert_eq!(preview.product.version, env!("CARGO_PKG_VERSION"));
    assert!(preview.exported_at.ends_with('Z'));
    assert_eq!(
        preview.database_migration_version,
        contractorcrm_lib::storage::latest_migration_version()
    );
    assert_eq!(
        preview.record_counts["contacts"] as usize,
        dump(&source)["contacts"].len()
    );
}

#[test]
fn import_leaves_a_restorable_pre_import_safety_backup() {
    let (temp, path, _source) = exported();
    let mut target = storage(temp.path(), "target");
    contractorcrm_lib::application::create_company(
        &mut target,
        serde_json::from_value(json!({"name": "Only in the live database", "kind": "client"}))
            .unwrap(),
    )
    .unwrap();
    let before = dump(&target);

    let report = import_archive(
        &mut target,
        &attachments(temp.path(), "target"),
        path.to_str().unwrap(),
    )
    .unwrap();
    assert_ne!(dump(&target), before);
    assert!(Path::new(&report.safety_backup_path).is_file());

    target.restore_from(&report.safety_backup_path).unwrap();
    assert_eq!(dump(&target), before, "the safety backup restores cleanly");
}

#[test]
fn a_file_that_is_not_an_archive_is_a_caller_error() {
    let temp = tempfile::tempdir().unwrap();
    let target = storage(temp.path(), "target");
    let path = temp.path().join("not-a-zip.zip");
    std::fs::write(&path, b"this is not a zip file").unwrap();

    let error = preview_archive_import(&target, path.to_str().unwrap()).unwrap_err();
    assert_eq!(error.kind(), "invalid_input");
    let missing = temp.path().join("missing.zip");
    let error = preview_archive_import(&target, missing.to_str().unwrap()).unwrap_err();
    assert_eq!(error.kind(), "invalid_input");
}

#[test]
fn inflated_manifest_counts_cannot_smuggle_an_empty_archive_past_preview() {
    let (temp, path, _source) = exported();
    // Empty every data file but leave the manifest claiming the original counts.
    let mut entries = read_entries(&path);
    for table in ARCHIVE_TABLES {
        set_table(&mut entries, table, &json!([]));
    }
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&entries["manifest.json"]).unwrap();
    let files = entries
        .iter()
        .filter(|(name, _)| name.as_str() != "manifest.json")
        .map(|(name, bytes)| {
            json!({"path": name, "sha256": sha256_hex(bytes), "bytes": bytes.len()})
        })
        .collect::<Vec<_>>();
    manifest["files"] = json!(files); // checksums honest, counts inflated
    entries.insert(
        "manifest.json".into(),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    );
    let tampered = temp.path().join("hollow.zip");
    write_entries(&tampered, &entries);

    let mut target = storage(temp.path(), "target");
    let before = dump(&target);
    let preview = preview_archive_import(&target, tampered.to_str().unwrap()).unwrap();
    assert!(
        issue_codes(&preview).contains(&"record_count_mismatch"),
        "{preview:?}"
    );
    // The preview reports what the archive actually holds, not the claim.
    assert_eq!(preview.record_counts["contacts"], 0);

    let error = import_archive(
        &mut target,
        &attachments(temp.path(), "target"),
        tampered.to_str().unwrap(),
    )
    .unwrap_err();
    assert_eq!(error.kind(), "validation_failed");
    assert_eq!(dump(&target), before, "the wipe never happens");
}

#[test]
fn constraint_violations_are_caught_by_the_preview_dry_run() {
    let (temp, path, _source) = exported();

    // A second tag with the same label breaks the case-insensitive unique index.
    let duplicate_label = repack(temp.path(), &path, "dup-tag.zip", |entries| {
        let mut tags = table_rows(entries, "tags");
        let mut clone = tags[0].clone();
        clone["id"] = json!("tag-duplicate-label");
        clone["label"] = json!(tags[0]["label"].as_str().unwrap().to_uppercase());
        tags.as_array_mut().unwrap().push(clone);
        set_table(entries, "tags", &tags);
    });
    // A negative amount breaks the opportunities CHECK.
    let negative_value = repack(temp.path(), &path, "negative.zip", |entries| {
        let mut opportunities = table_rows(entries, "opportunities");
        opportunities[0]["valueMinor"] = json!(-1);
        set_table(entries, "opportunities", &opportunities);
    });

    for tampered in [duplicate_label, negative_value] {
        let mut target = storage(temp.path(), "target");
        let before = dump(&target);
        let preview = preview_archive_import(&target, tampered.to_str().unwrap()).unwrap();
        assert!(
            issue_codes(&preview).contains(&"constraint_violation"),
            "{tampered:?}: {preview:?}"
        );
        let error = import_archive(
            &mut target,
            &attachments(temp.path(), "target"),
            tampered.to_str().unwrap(),
        )
        .unwrap_err();
        assert_eq!(error.kind(), "validation_failed");
        assert_eq!(dump(&target), before);
        std::fs::remove_file(&tampered).unwrap();
    }
}

#[test]
fn an_oversized_entry_is_refused_without_being_buffered() {
    let (temp, path, _source) = exported();
    let mut entries = read_entries(&path);
    // Highly compressible, so the archive on disk stays tiny — the classic
    // ratio bomb. 257 MiB of zeros is past the per-entry cap.
    entries.insert("data/companies.json".into(), vec![b' '; 257 * 1024 * 1024]);
    let tampered = temp.path().join("bomb.zip");
    write_entries(&tampered, &entries);
    assert!(
        std::fs::metadata(&tampered).unwrap().len() < 5 * 1024 * 1024,
        "the bomb must be cheap on disk"
    );

    let mut target = storage(temp.path(), "target");
    let preview = preview_archive_import(&target, tampered.to_str().unwrap()).unwrap();
    assert!(
        issue_codes(&preview).contains(&"entry_too_large"),
        "{preview:?}"
    );
    let error = import_archive(
        &mut target,
        &attachments(temp.path(), "target"),
        tampered.to_str().unwrap(),
    )
    .unwrap_err();
    assert_eq!(error.kind(), "validation_failed");
}

#[test]
fn an_asset_file_without_an_attachments_row_is_refused() {
    let (temp, path, _source) = exported();
    let tampered = repack(temp.path(), &path, "asset.zip", |entries| {
        entries.insert("assets/photo.jpg".into(), b"not really a photo".to_vec());
    });

    let mut target = storage(temp.path(), "target");
    let preview = preview_archive_import(&target, tampered.to_str().unwrap()).unwrap();
    assert!(
        issue_codes(&preview).contains(&"unexpected_asset"),
        "{preview:?}"
    );
    let error = import_archive(
        &mut target,
        &attachments(temp.path(), "target"),
        tampered.to_str().unwrap(),
    )
    .unwrap_err();
    assert_eq!(error.kind(), "validation_failed");
}

#[test]
fn issue_reporting_is_capped_so_payloads_stay_bounded() {
    let (temp, path, _source) = exported();
    let tampered = repack(temp.path(), &path, "noisy.zip", |entries| {
        let template = table_rows(entries, "contacts")[0].clone();
        let mut contacts = Vec::new();
        for index in 0..150 {
            let mut row = template.clone();
            row["id"] = json!(format!("contact-{index}"));
            row["surpriseColumn"] = json!("noise");
            contacts.push(row);
        }
        set_table(entries, "contacts", &json!(contacts));
    });

    let target = storage(temp.path(), "target");
    let preview = preview_archive_import(&target, tampered.to_str().unwrap()).unwrap();
    assert_eq!(preview.issues.len(), 101, "100 issues plus the summary");
    assert_eq!(preview.issues.last().unwrap().code, "too_many_issues");
    assert!(
        preview.issues.last().unwrap().message.contains("more"),
        "{:?}",
        preview.issues.last()
    );
}

#[test]
fn an_archive_without_a_usable_pipeline_is_refused() {
    let (temp, path, _source) = exported();
    // Consistent but empty: no dangling references, no pipeline either.
    let tampered = repack(temp.path(), &path, "empty.zip", |entries| {
        for table in ARCHIVE_TABLES {
            set_table(entries, table, &json!([]));
        }
    });

    let mut target = storage(temp.path(), "target");
    let preview = preview_archive_import(&target, tampered.to_str().unwrap()).unwrap();
    assert!(
        issue_codes(&preview).contains(&"missing_pipeline"),
        "{preview:?}"
    );
    let error = import_archive(
        &mut target,
        &attachments(temp.path(), "target"),
        tampered.to_str().unwrap(),
    )
    .unwrap_err();
    assert_eq!(error.kind(), "validation_failed");

    // A pipeline whose stages cannot express won or lost is just as unusable.
    let no_kinds = repack(temp.path(), &path, "kinds.zip", |entries| {
        let mut stages = table_rows(entries, "stages");
        let open_only = stages
            .as_array()
            .unwrap()
            .iter()
            .filter(|stage| stage["kind"] == "open")
            .cloned()
            .collect::<Vec<_>>();
        stages = json!(open_only);
        set_table(entries, "stages", &stages);
        // Everything that pointed at the removed stages goes too.
        for table in [
            "opportunities",
            "stage_history",
            "record_tags",
            "attachments",
        ] {
            set_table(entries, table, &json!([]));
        }
        for table in ["activities", "tasks", "custom_field_values"] {
            set_table(entries, table, &json!([]));
        }
    });
    let preview = preview_archive_import(&target, no_kinds.to_str().unwrap()).unwrap();
    assert!(
        issue_codes(&preview).contains(&"missing_stage_kind"),
        "{preview:?}"
    );
}

#[test]
fn a_refused_import_leaves_no_orphan_safety_backup() {
    let (temp, path, _source) = exported();
    let tampered = repack(temp.path(), &path, "dangling.zip", |entries| {
        let mut contacts = table_rows(entries, "contacts");
        contacts[0]["companyId"] = json!("company-that-does-not-exist");
        set_table(entries, "contacts", &contacts);
    });

    let mut target = storage(temp.path(), "target");
    let directory = target.database_path().parent().unwrap().to_path_buf();
    import_archive(
        &mut target,
        &attachments(temp.path(), "target"),
        tampered.to_str().unwrap(),
    )
    .unwrap_err();

    let backups = std::fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".bak"))
        .collect::<Vec<_>>();
    assert!(backups.is_empty(), "{backups:?}");
}

#[test]
fn exports_refuse_to_overwrite_the_live_database() {
    let temp = tempfile::tempdir().unwrap();
    let mut target = storage(temp.path(), "live");
    let database = target.database_path().to_path_buf();
    let directory = database.parent().unwrap().to_path_buf();
    let name = database.file_name().unwrap().to_string_lossy().into_owned();

    for destination in [
        database.clone(),
        directory.join(format!("{name}-wal")),
        directory.join(format!("{name}-shm")),
        directory.join(format!("{name}.pre-import-20260818T000000000Z.bak")),
    ] {
        let path = destination.to_str().unwrap();
        for error in [
            export_archive(&mut target, &attachments(temp.path(), "live"), path, true).unwrap_err(),
            contractorcrm_lib::application::export_contacts_csv(&mut target, path, true)
                .unwrap_err(),
            contractorcrm_lib::application::export_opportunities_csv(&mut target, path, true)
                .unwrap_err(),
        ] {
            assert_eq!(error.kind(), "validation_failed", "{destination:?}");
            assert!(error.to_string().contains("live database"), "{error}");
        }
    }
    // The database still opens and works after the refusals.
    assert!(database.is_file());
    contractorcrm_lib::application::list_contacts(&target, false).unwrap();
}

#[test]
fn a_failed_csv_export_keeps_the_previous_file() {
    let temp = tempfile::tempdir().unwrap();
    let mut source = storage(temp.path(), "source");
    populated(
        &mut source,
        &attachments(temp.path(), "source"),
        &temp.path().join("inbox"),
    );
    let path = temp.path().join("contacts.csv");
    contractorcrm_lib::application::export_contacts_csv(&mut source, path.to_str().unwrap(), false)
        .unwrap();
    let good = std::fs::read(&path).unwrap();
    assert!(!good.is_empty());

    // Break the query the export depends on, then re-export over the file.
    source
        .connection()
        .execute_batch(
            "DROP TABLE custom_field_values;
             DROP TABLE custom_field_options;
             DROP TABLE custom_field_defs;",
        )
        .unwrap();
    contractorcrm_lib::application::export_contacts_csv(&mut source, path.to_str().unwrap(), true)
        .unwrap_err();
    assert_eq!(
        std::fs::read(&path).unwrap(),
        good,
        "a failed export must not truncate the previous one"
    );
}

#[test]
fn another_products_archive_is_refused_with_one_clear_issue() {
    let (temp, path, _source) = exported();
    let tampered = repack(temp.path(), &path, "foreign.zip", |entries| {
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&entries["manifest.json"]).unwrap();
        manifest["product"]["name"] = json!("SomeOtherCRM");
        entries.insert(
            "manifest.json".into(),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        );
    });

    let mut target = storage(temp.path(), "target");
    let preview = preview_archive_import(&target, tampered.to_str().unwrap()).unwrap();
    assert_eq!(issue_codes(&preview), ["wrong_product"], "{preview:?}");
    let error = import_archive(
        &mut target,
        &attachments(temp.path(), "target"),
        tampered.to_str().unwrap(),
    )
    .unwrap_err();
    assert_eq!(error.kind(), "validation_failed");
}

#[test]
fn an_archive_carries_managed_attachment_files_and_replaces_the_targets() {
    let (temp, path, source) = exported();
    let expected = dump(&source);
    assert_eq!(expected["attachments"].len(), 2);
    // Every managed file is packed next to its row.
    let entries = read_entries(&path);
    let assets = entries
        .keys()
        .filter(|name| name.starts_with("assets/"))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(assets.len(), 2, "{assets:?}");
    assert!(
        assets.iter().any(|name| name.ends_with("/cedar-quote.pdf")),
        "{assets:?}"
    );

    // The target starts with a managed file of its own; the import replaces it.
    let mut target = storage(temp.path(), "target");
    let target_store = attachments(temp.path(), "target");
    let stale = target_store.root().join("stale-attachment");
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(stale.join("gone.txt"), b"replaced by the import").unwrap();

    import_archive(&mut target, &target_store, path.to_str().unwrap()).unwrap();

    assert_eq!(dump(&target), expected);
    assert!(!stale.exists(), "orphaned managed files are cleared");
    let mut located = Vec::new();
    for id in ids(&target, "SELECT id FROM attachments ORDER BY created_at") {
        let location =
            contractorcrm_lib::attachments::attachment_path(&target, &target_store, &id).unwrap();
        assert!(location.exists, "{location:?}");
        located.push(std::fs::read(&location.path).unwrap());
    }
    assert!(
        located.contains(&b"measurements and gate swing".to_vec()),
        "attachment bytes survive the round trip"
    );
    // No staging directory is left behind.
    let leftovers = std::fs::read_dir(target_store.root())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(".import-staging-"))
        .collect::<Vec<_>>();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn tampered_attachment_bytes_are_refused_by_the_manifest_and_by_the_row() {
    let (temp, path, _source) = exported();
    let asset_name = read_entries(&path)
        .keys()
        .find(|name| name.ends_with("/cedar-quote.pdf"))
        .unwrap()
        .clone();

    // Repacked without re-signing: the manifest checksum catches it.
    let mut entries = read_entries(&path);
    entries.insert(asset_name.clone(), b"%PDF-1.4 swapped quote".to_vec());
    let unsigned = temp.path().join("asset-unsigned.zip");
    write_entries(&unsigned, &entries);

    // Re-signed with the same length: only the row's checksum can catch it.
    let same_length = repack(temp.path(), &path, "asset-resigned.zip", |entries| {
        entries.insert(asset_name.clone(), b"%PDF-1.4 CEDAR quote".to_vec());
    });
    // Re-signed with a different length: the row's size catches it first.
    let different_length = repack(temp.path(), &path, "asset-longer.zip", |entries| {
        entries.insert(asset_name.clone(), b"%PDF-1.4 much longer quote".to_vec());
    });

    for (tampered, code) in [
        (unsigned, "checksum_mismatch"),
        (same_length, "attachment_checksum_mismatch"),
        (different_length, "attachment_size_mismatch"),
    ] {
        let mut target = storage(temp.path(), "target");
        let before = dump(&target);
        let preview = preview_archive_import(&target, tampered.to_str().unwrap()).unwrap();
        assert!(issue_codes(&preview).contains(&code), "{code}: {preview:?}");
        let error = import_archive(
            &mut target,
            &attachments(temp.path(), "target"),
            tampered.to_str().unwrap(),
        )
        .unwrap_err();
        assert_eq!(error.kind(), "validation_failed");
        assert_eq!(dump(&target), before);
    }
}

#[test]
fn an_attachments_row_without_its_file_is_refused() {
    let (temp, path, _source) = exported();
    let tampered = repack(temp.path(), &path, "no-asset.zip", |entries| {
        entries.retain(|name, _| !name.ends_with("/cedar-quote.pdf"));
    });

    let mut target = storage(temp.path(), "target");
    let preview = preview_archive_import(&target, tampered.to_str().unwrap()).unwrap();
    assert!(
        issue_codes(&preview).contains(&"attachment_file_missing"),
        "{preview:?}"
    );
    let error = import_archive(
        &mut target,
        &attachments(temp.path(), "target"),
        tampered.to_str().unwrap(),
    )
    .unwrap_err();
    assert_eq!(error.kind(), "validation_failed");
    // Nothing was staged on the way to the refusal.
    assert!(!attachments(temp.path(), "target").root().exists());
}

#[test]
fn an_archive_written_before_attachments_existed_still_imports() {
    let (temp, path, _source) = exported();
    // Exactly what a migration-9 export looks like: no attachments table file,
    // no assets, and a manifest that predates the table.
    let older = repack(temp.path(), &path, "migration-9.zip", |entries| {
        entries.retain(|name, _| name != "data/attachments.json" && !name.starts_with("assets/"));
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&entries["manifest.json"]).unwrap();
        manifest["databaseMigrationVersion"] = json!(9);
        entries.insert(
            "manifest.json".into(),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        );
    });

    let mut target = storage(temp.path(), "target");
    let preview = preview_archive_import(&target, older.to_str().unwrap()).unwrap();
    assert_eq!(preview.issues, vec![], "{preview:?}");
    assert!(!preview.record_counts.contains_key("attachments"));

    let report = import_archive(
        &mut target,
        &attachments(temp.path(), "target"),
        older.to_str().unwrap(),
    )
    .unwrap();
    assert_eq!(report.record_counts["attachments"], 0);
    assert!(dump(&target)["attachments"].is_empty());
    assert!(!dump(&target)["contacts"].is_empty());
}

#[test]
fn an_export_refuses_to_write_an_archive_missing_a_managed_file() {
    let temp = tempfile::tempdir().unwrap();
    let mut source = storage(temp.path(), "source");
    let store = attachments(temp.path(), "source");
    populated(&mut source, &store, &temp.path().join("inbox"));
    let id = ids(&source, "SELECT id FROM attachments ORDER BY created_at")
        .first()
        .unwrap()
        .clone();
    std::fs::remove_dir_all(store.root().join(&id)).unwrap();

    let path = temp.path().join("broken.zip");
    let error = export_archive(&mut source, &store, path.to_str().unwrap(), false).unwrap_err();
    assert_eq!(error.kind(), "validation_failed");
    assert!(error.to_string().contains("missing its file"), "{error}");
    assert!(!path.exists(), "a refused export writes nothing");
}

/// Ids from a single-column query, in query order.
fn ids(storage: &Storage, sql: &str) -> Vec<String> {
    storage
        .connection()
        .prepare(sql)
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

#[test]
fn an_archive_whose_row_records_a_foreign_path_is_refused() {
    let (temp, path, _source) = exported();
    let tampered = repack(temp.path(), &path, "poisoned-path.zip", |entries| {
        let mut rows = table_rows(entries, "attachments");
        rows[0]["relativePath"] = serde_json::json!("../../outside.txt");
        set_table(entries, "attachments", &rows);
    });

    let mut target = storage(temp.path(), "target");
    let before = dump(&target);
    let preview = preview_archive_import(&target, tampered.to_str().unwrap()).unwrap();
    assert!(
        issue_codes(&preview).contains(&"attachment_path_mismatch"),
        "{preview:?}"
    );
    let error = import_archive(
        &mut target,
        &attachments(temp.path(), "target"),
        tampered.to_str().unwrap(),
    )
    .unwrap_err();
    assert_eq!(error.kind(), "validation_failed");
    assert_eq!(dump(&target), before);
}

#[test]
fn a_poisoned_relative_path_never_escapes_the_attachments_root() {
    let temp = tempfile::tempdir().unwrap();
    let mut source = storage(temp.path(), "source");
    let store = attachments(temp.path(), "source");
    populated(&mut source, &store, &temp.path().join("inbox"));

    // A sentinel outside the managed root that a poisoned row points at.
    let sentinel = temp.path().join("outside.txt");
    std::fs::write(&sentinel, b"keep me").unwrap();
    source
        .connection()
        .execute(
            "UPDATE attachments SET relative_path = '../../outside.txt'
             WHERE id = (SELECT id FROM attachments ORDER BY id LIMIT 1)",
            [],
        )
        .unwrap();
    let (id, version): (String, i64) = source
        .connection()
        .query_row(
            "SELECT id, version FROM attachments WHERE relative_path = '../../outside.txt'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();

    // Every consumer of the stored path refuses instead of leaving the root.
    let location = contractorcrm_lib::attachments::attachment_path(&source, &store, &id);
    assert_eq!(location.unwrap_err().kind(), "invalid_stored_data");
    // Removal only ever touches the managed <root>/<id> directory, so it
    // succeeds without following the poisoned path.
    contractorcrm_lib::attachments::remove_attachment(
        &mut source,
        &store,
        contractorcrm_lib::attachments::RemoveAttachmentRequest {
            actor: Default::default(),
            attachment_id: id,
            expected_version: version,
        },
    )
    .unwrap();
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"keep me");
    // Re-poison another row so export still has one to trip over.
    source
        .connection()
        .execute(
            "UPDATE attachments SET relative_path = '../../outside.txt'
             WHERE id = (SELECT id FROM attachments ORDER BY id LIMIT 1)",
            [],
        )
        .unwrap();
    let export = export_archive(
        &mut source,
        &store,
        temp.path().join("poisoned-export.zip").to_str().unwrap(),
        false,
    );
    assert_eq!(export.unwrap_err().kind(), "invalid_stored_data");

    assert_eq!(std::fs::read(&sentinel).unwrap(), b"keep me");
}
