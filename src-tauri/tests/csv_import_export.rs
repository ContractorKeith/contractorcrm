use contractorcrm_lib::application::{
    create_contact, create_custom_field_def, create_opportunity, create_tag, export_contacts_csv,
    export_opportunities_csv, get_contact, import_contacts, list_contacts, preview_contact_import,
    set_record_metadata, ContactImportMapping, CreateContactRequest, CreateCustomFieldDefRequest,
    CreateOpportunityRequest, CreateTagRequest, CustomFieldValueInput, ImportContactsRequest,
    SavedViewEntityType, SetRecordMetadataRequest,
};
use contractorcrm_lib::domain::Actor;
use contractorcrm_lib::storage::Storage;
use std::path::PathBuf;

/// Fresh storage plus a temp dir that keeps CSV fixtures alive for the test.
fn fixture() -> (tempfile::TempDir, Storage) {
    let temp = tempfile::tempdir().unwrap();
    let storage = Storage::open_in_app_data(temp.path()).unwrap();
    (temp, storage)
}

fn write_csv(temp: &tempfile::TempDir, name: &str, body: &str) -> PathBuf {
    let path = temp.path().join(name);
    std::fs::write(&path, body).unwrap();
    path
}

fn text(path: &PathBuf) -> String {
    std::fs::read_to_string(path).unwrap()
}

fn command_log_actors(storage: &Storage, entity_type: &str) -> Vec<String> {
    let mut statement = storage
        .connection()
        .prepare("SELECT actor FROM command_log WHERE entity_type = ?1 ORDER BY created_at, id")
        .unwrap();
    statement
        .query_map([entity_type], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<String>>>()
        .unwrap()
}

fn contact_by_display_name(
    storage: &Storage,
    display_name: &str,
) -> contractorcrm_lib::domain::Contact {
    list_contacts(storage, false)
        .unwrap()
        .into_iter()
        .map(|item| item.contact)
        .find(|contact| contact.display_name == display_name)
        .unwrap_or_else(|| panic!("no contact named {display_name}"))
}

const SAMPLE: &str = "External ID,First Name,Last Name,Company Name,Email,Phone Number,City,Tags\n\
crm-1,Dana,Reyes,Ridgeline Fence,dana@ridgeline.test,555-0100,Ocala,VIP;Repeat\n\
crm-2,Sam,Ortiz,Ridgeline Fence,sam@ridgeline.test,555-0101,Ocala,\n";

#[test]
fn preview_guesses_a_mapping_and_reports_row_issues_without_writing() {
    let (temp, storage) = fixture();
    let path = write_csv(
        &temp,
        "contacts.csv",
        "External ID,First Name,Last Name,Email,Role\n\
         crm-1,Dana,Reyes,dana@ridgeline.test,owner\n\
         crm-2,,,sam@ridgeline.test,owner\n\
         crm-3,Sam,Ortiz,sam@ridgeline.test,chief_of_vibes\n",
    );

    let preview = preview_contact_import(path.to_str().unwrap(), None).unwrap();
    assert_eq!(
        preview.headers,
        ["External ID", "First Name", "Last Name", "Email", "Role"]
    );
    assert_eq!(preview.row_count, 3);
    assert_eq!(preview.sample_rows.len(), 3);
    assert_eq!(preview.mapping.external_id.as_deref(), Some("External ID"));
    assert_eq!(preview.mapping.first_name.as_deref(), Some("First Name"));
    assert_eq!(preview.mapping.last_name.as_deref(), Some("Last Name"));
    assert_eq!(preview.mapping.email.as_deref(), Some("Email"));
    assert_eq!(preview.mapping.role.as_deref(), Some("Role"));
    assert!(preview.mapping.phone.is_none());

    // Row 3 has no name, row 4 has an unknown role; both are flagged by line.
    let lines = preview
        .issues
        .iter()
        .map(|issue| issue.line)
        .collect::<Vec<_>>();
    assert_eq!(lines, [3, 4]);
    assert!(preview.issues[0].reason.contains("displayName"));
    assert!(preview.issues[1].reason.contains("role"));

    // Preview is read-only.
    assert!(list_contacts(&storage, true).unwrap().is_empty());
}

#[test]
fn preview_honors_an_explicit_mapping_and_rejects_unknown_columns() {
    let (temp, _storage) = fixture();
    let path = write_csv(&temp, "contacts.csv", "Person,Where\nDana Reyes,Ocala\n");

    let mapping = ContactImportMapping {
        display_name: Some("Person".into()),
        city: Some("Where".into()),
        ..Default::default()
    };
    let preview = preview_contact_import(path.to_str().unwrap(), Some(mapping)).unwrap();
    assert_eq!(preview.mapping.display_name.as_deref(), Some("Person"));
    assert!(preview.issues.is_empty());

    let bad = ContactImportMapping {
        display_name: Some("Missing".into()),
        ..Default::default()
    };
    let error = preview_contact_import(path.to_str().unwrap(), Some(bad)).unwrap_err();
    assert_eq!(error.kind(), "invalid_input");
}

#[test]
fn preview_samples_at_most_fifty_rows_but_counts_them_all() {
    let (temp, _storage) = fixture();
    let mut body = String::from("Name\n");
    for index in 0..75 {
        body.push_str(&format!("Contact {index}\n"));
    }
    let path = write_csv(&temp, "many.csv", &body);

    let preview = preview_contact_import(path.to_str().unwrap(), None).unwrap();
    assert_eq!(preview.row_count, 75);
    assert_eq!(preview.sample_rows.len(), 50);
}

#[test]
fn import_creates_contacts_companies_and_tags_and_logs_as_import() {
    let (temp, mut storage) = fixture();
    let path = write_csv(&temp, "contacts.csv", SAMPLE);
    let mapping = preview_contact_import(path.to_str().unwrap(), None)
        .unwrap()
        .mapping;

    let summary = import_contacts(
        &mut storage,
        ImportContactsRequest {
            actor: Actor::Import,
            path: path.to_str().unwrap().into(),
            mapping,
        },
    )
    .unwrap();
    assert_eq!(summary.created, 2);
    assert_eq!(summary.updated, 0);
    assert!(summary.skipped.is_empty());

    let dana = contact_by_display_name(&storage, "Dana Reyes");
    assert_eq!(dana.city.as_deref(), Some("Ocala"));
    assert_eq!(dana.channels.len(), 2);
    assert!(dana
        .channels
        .iter()
        .any(|channel| { channel.value == "dana@ridgeline.test" && channel.preferred }));
    assert!(dana
        .channels
        .iter()
        .any(|channel| channel.value == "555-0100"));

    // The company was created once and shared by both rows.
    let companies = contractorcrm_lib::application::list_companies(&storage, true).unwrap();
    assert_eq!(companies.len(), 1);
    assert_eq!(companies[0].name, "Ridgeline Fence");
    assert_eq!(dana.company_id.as_deref(), Some(companies[0].id.as_str()));

    // Tags were created from the semicolon list and linked to the row's contact.
    let tags = contractorcrm_lib::application::list_tags(&storage, true).unwrap();
    let mut labels = tags.iter().map(|tag| tag.label.clone()).collect::<Vec<_>>();
    labels.sort();
    assert_eq!(labels, ["Repeat", "VIP"]);
    let linked: i64 = storage
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM record_tags WHERE entity_type='contact' AND record_id=?1",
            [&dana.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(linked, 2);

    // Every write is attributed to the import actor.
    assert_eq!(
        command_log_actors(&storage, "contact"),
        ["import", "import"]
    );
    assert_eq!(command_log_actors(&storage, "company"), ["import"]);
    assert_eq!(command_log_actors(&storage, "tag"), ["import", "import"]);

    // Imported contacts are searchable through the projection.
    let hits = contractorcrm_lib::application::search_records(&storage, "Reyes".into(), None, None)
        .unwrap();
    assert!(hits.iter().any(|hit| hit.entity_id == dana.id));
}

#[test]
fn import_updates_by_external_id_and_skips_invalid_rows() {
    let (temp, mut storage) = fixture();
    let first = write_csv(&temp, "first.csv", SAMPLE);
    let mapping = preview_contact_import(first.to_str().unwrap(), None)
        .unwrap()
        .mapping;
    import_contacts(
        &mut storage,
        ImportContactsRequest {
            actor: Actor::Import,
            path: first.to_str().unwrap().into(),
            mapping: mapping.clone(),
        },
    )
    .unwrap();
    let before = contact_by_display_name(&storage, "Dana Reyes");

    let second = write_csv(
        &temp,
        "second.csv",
        "External ID,First Name,Last Name,Company Name,Email,Phone Number,City,Tags\n\
         crm-1,Dana,Reyes-Kim,Ridgeline Fence,dana@ridgeline.test,555-0199,Gainesville,VIP\n\
         ,,,Ridgeline Fence,nameless@ridgeline.test,,,\n\
         crm-3,Alex,Nguyen,Coastal Gates,alex@coastal.test,555-0102,Tampa,\n",
    );
    let summary = import_contacts(
        &mut storage,
        ImportContactsRequest {
            actor: Actor::Import,
            path: second.to_str().unwrap().into(),
            mapping,
        },
    )
    .unwrap();
    assert_eq!(summary.created, 1);
    assert_eq!(summary.updated, 1);
    assert_eq!(summary.skipped.len(), 1);
    assert_eq!(summary.skipped[0].line, 3);

    let after = get_contact(&storage, &before.id).unwrap();
    assert_eq!(after.display_name, "Dana Reyes-Kim");
    assert_eq!(after.city.as_deref(), Some("Gainesville"));
    assert_eq!(after.version, before.version + 1);
    // Channels are additive: the new phone joins the old one, the unchanged
    // email is not duplicated.
    let mut values = after
        .channels
        .iter()
        .map(|channel| channel.value.clone())
        .collect::<Vec<_>>();
    values.sort();
    assert_eq!(values, ["555-0100", "555-0199", "dana@ridgeline.test"]);
    // A second company was created for the new row only.
    assert_eq!(
        contractorcrm_lib::application::list_companies(&storage, true)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(list_contacts(&storage, true).unwrap().len(), 3);
}

#[test]
fn import_is_atomic_and_leaves_nothing_behind_when_it_fails() {
    let (temp, mut storage) = fixture();
    let path = write_csv(&temp, "contacts.csv", SAMPLE);
    let missing = ContactImportMapping {
        display_name: Some("Nope".into()),
        ..Default::default()
    };
    let error = import_contacts(
        &mut storage,
        ImportContactsRequest {
            actor: Actor::Import,
            path: path.to_str().unwrap().into(),
            mapping: missing,
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), "invalid_input");
    assert!(list_contacts(&storage, true).unwrap().is_empty());
    assert!(
        contractorcrm_lib::application::list_companies(&storage, true)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn import_handles_quoted_commas_newlines_and_blank_rows() {
    let (temp, mut storage) = fixture();
    let path = write_csv(
        &temp,
        "messy.csv",
        "Name,Notes,Company Name\n\
         \"Reyes, Dana\",\"Line one\nLine two, with comma\",\"Ridgeline \"\"The Fence\"\" Co\"\n\
         \n\
         Sam Ortiz,plain,Ridgeline \"\"The Fence\"\" Co\n",
    );
    let mapping = preview_contact_import(path.to_str().unwrap(), None)
        .unwrap()
        .mapping;
    assert_eq!(mapping.display_name.as_deref(), Some("Name"));
    assert_eq!(mapping.company.as_deref(), Some("Company Name"));

    let summary = import_contacts(
        &mut storage,
        ImportContactsRequest {
            actor: Actor::Import,
            path: path.to_str().unwrap().into(),
            mapping,
        },
    )
    .unwrap();
    assert_eq!(summary.created, 2);
    let dana = contact_by_display_name(&storage, "Reyes, Dana");
    assert_eq!(
        dana.notes.as_deref(),
        Some("Line one\nLine two, with comma")
    );

    // Exports quote the same values back out and re-import unchanged.
    let export_path = temp.path().join("out.csv");
    export_contacts_csv(&mut storage, export_path.to_str().unwrap(), false).unwrap();
    let exported = text(&export_path);
    assert!(exported.contains("\"Reyes, Dana\""));
    assert!(exported.contains("\"Line one\nLine two, with comma\""));
}

#[test]
fn contact_export_carries_metadata_columns_and_round_trips_without_creates() {
    let (temp, mut storage) = fixture();
    let contact = create_contact(
        &mut storage,
        serde_json::from_value::<CreateContactRequest>(serde_json::json!({
            "firstName": "Dana",
            "lastName": "Reyes",
            "kind": "client",
            "city": "Ocala",
            "notes": "Repeat client",
            "channels": [
                {"kind": "email", "value": "second@ridgeline.test"},
                {"kind": "email", "value": "dana@ridgeline.test", "preferred": true},
                {"kind": "phone", "value": "555-0100", "preferred": true}
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    let tag = create_tag(
        &mut storage,
        CreateTagRequest {
            actor: Actor::User,
            label: "VIP".into(),
            color_role: None,
        },
    )
    .unwrap();
    let field = create_custom_field_def(
        &mut storage,
        CreateCustomFieldDefRequest {
            actor: Actor::User,
            entity_type: SavedViewEntityType::Contact,
            label: "Gate Code".into(),
            field_type: "text".into(),
            options: vec![],
        },
    )
    .unwrap();
    set_record_metadata(
        &mut storage,
        SetRecordMetadataRequest {
            actor: Actor::User,
            entity_type: SavedViewEntityType::Contact,
            record_id: contact.id.clone(),
            expected_version: contact.version,
            tag_ids: vec![tag.id.clone()],
            values: vec![CustomFieldValueInput {
                definition_id: field.id.clone(),
                text_value: Some("#4821".into()),
                number_value: None,
                date_value: None,
                option_id: None,
            }],
        },
    )
    .unwrap();

    let export_path = temp.path().join("contacts.csv");
    let report = export_contacts_csv(&mut storage, export_path.to_str().unwrap(), false).unwrap();
    assert_eq!(report.row_count, 1);
    let exported = text(&export_path);
    let header = exported.lines().next().unwrap();
    assert!(header.starts_with("id,external_id,first_name,last_name,display_name"));
    assert!(header.contains(",tags,Gate Code,created_at,updated_at"));
    let row = exported.lines().nth(1).unwrap();
    assert!(row.contains("Dana Reyes"));
    assert!(row.contains("dana@ridgeline.test")); // preferred email wins
    assert!(!row.contains("second@ridgeline.test"));
    assert!(row.contains("VIP"));
    assert!(row.contains("#4821"));

    // Re-importing the export matches the same records: updates, never creates.
    let mapping = preview_contact_import(export_path.to_str().unwrap(), None)
        .unwrap()
        .mapping;
    assert_eq!(mapping.external_id.as_deref(), Some("external_id"));
    let summary = import_contacts(
        &mut storage,
        ImportContactsRequest {
            actor: Actor::Import,
            path: export_path.to_str().unwrap().into(),
            mapping,
        },
    )
    .unwrap();
    assert_eq!(summary.created, 0);
    assert_eq!(summary.updated, 1);
    assert!(summary.skipped.is_empty());
    assert_eq!(list_contacts(&storage, true).unwrap().len(), 1);
    let reloaded = get_contact(&storage, &contact.id).unwrap();
    assert_eq!(reloaded.display_name, "Dana Reyes");
    assert_eq!(reloaded.notes.as_deref(), Some("Repeat client"));
    // The secondary email the export could not carry must survive re-import.
    let mut values = reloaded
        .channels
        .iter()
        .map(|channel| channel.value.clone())
        .collect::<Vec<_>>();
    values.sort();
    assert_eq!(
        values,
        ["555-0100", "dana@ridgeline.test", "second@ridgeline.test"]
    );
    assert_eq!(reloaded.channels.len(), contact.channels.len());
}

#[test]
fn opportunity_export_writes_major_units_stage_and_metadata_columns() {
    let (temp, mut storage) = fixture();
    let contact = create_contact(
        &mut storage,
        serde_json::from_value::<CreateContactRequest>(serde_json::json!({
            "displayName": "Dana Reyes", "kind": "client"
        }))
        .unwrap(),
    )
    .unwrap();
    let opportunity = create_opportunity(
        &mut storage,
        serde_json::from_value::<CreateOpportunityRequest>(serde_json::json!({
            "name": "Back yard fence",
            "contactId": contact.id,
            "valueMinor": 1234567,
            "currencyCode": "USD",
            "probabilityPercent": 60,
            "expectedCloseDate": "2026-09-01",
            "source": "referral",
            "sourceLabel": "Neighbor, next door"
        }))
        .unwrap(),
    )
    .unwrap();
    let field = create_custom_field_def(
        &mut storage,
        CreateCustomFieldDefRequest {
            actor: Actor::User,
            entity_type: SavedViewEntityType::Opportunity,
            label: "Linear Feet".into(),
            field_type: "number".into(),
            options: vec![],
        },
    )
    .unwrap();
    set_record_metadata(
        &mut storage,
        SetRecordMetadataRequest {
            actor: Actor::User,
            entity_type: SavedViewEntityType::Opportunity,
            record_id: opportunity.id.clone(),
            expected_version: opportunity.version,
            tag_ids: vec![],
            values: vec![CustomFieldValueInput {
                definition_id: field.id.clone(),
                text_value: None,
                number_value: Some(320.0),
                date_value: None,
                option_id: None,
            }],
        },
    )
    .unwrap();

    let export_path = temp.path().join("opportunities.csv");
    let report =
        export_opportunities_csv(&mut storage, export_path.to_str().unwrap(), false).unwrap();
    assert_eq!(report.row_count, 1);
    let exported = text(&export_path);
    assert_eq!(
        exported.lines().next().unwrap(),
        "id,name,contact_display_name,company,stage,value,currency_code,probability_percent,\
         expected_close_date,source,source_label,tags,Linear Feet,created_at,updated_at"
    );
    let row = exported.lines().nth(1).unwrap();
    assert!(row.contains(",12345.67,USD,60,2026-09-01,referral,"));
    assert!(row.contains("\"Neighbor, next door\""));
    assert!(row.contains("Lead")); // seeded first open stage
    assert!(row.contains(",320,"));
}

/// APFS and NTFS are case-insensitive, so an uppercase spelling of the live
/// database still names the live database — and a CSV export onto it would
/// destroy the CRM.
#[test]
fn export_refuses_a_differently_cased_spelling_of_the_live_database() {
    let (temp, mut storage) = fixture();
    let database = temp.path().join("contractorcrm.sqlite3");
    let before = std::fs::read(&database).unwrap();

    for destination in [
        temp.path().join("CONTRACTORCRM.SQLITE3"),
        temp.path().join("ContractorCRM.Sqlite3-WAL"),
        temp.path()
            .join("CONTRACTORCRM.SQLITE3.20260819T101010000Z.bak"),
    ] {
        for overwrite in [false, true] {
            let error = export_contacts_csv(&mut storage, destination.to_str().unwrap(), overwrite)
                .unwrap_err();
            assert_eq!(error.kind(), "validation_failed", "{destination:?}");
            assert!(
                error.to_string().contains("live database"),
                "{destination:?}: {error}"
            );
        }
    }

    assert_eq!(std::fs::read(&database).unwrap(), before);
}

#[test]
fn export_creates_missing_directories_and_refuses_to_clobber_without_overwrite() {
    let (temp, mut storage) = fixture();
    create_contact(
        &mut storage,
        serde_json::from_value::<CreateContactRequest>(serde_json::json!({
            "displayName": "Dana Reyes", "kind": "client"
        }))
        .unwrap(),
    )
    .unwrap();

    let nested = temp.path().join("exports").join("contacts.csv");
    export_contacts_csv(&mut storage, nested.to_str().unwrap(), false).unwrap();
    assert!(nested.is_file());
    let first = text(&nested);

    create_contact(
        &mut storage,
        serde_json::from_value::<CreateContactRequest>(serde_json::json!({
            "displayName": "Sam Ortiz", "kind": "client"
        }))
        .unwrap(),
    )
    .unwrap();

    // An existing file is never silently replaced.
    let error = export_contacts_csv(&mut storage, nested.to_str().unwrap(), false).unwrap_err();
    assert_eq!(error.kind(), "validation_failed");
    assert!(error.to_string().contains("already exists"));
    assert_eq!(first, text(&nested));

    let report = export_contacts_csv(&mut storage, nested.to_str().unwrap(), true).unwrap();
    assert_eq!(report.row_count, 2);
    assert_ne!(first, text(&nested));

    // Opportunity exports guard the same way.
    let opportunities = temp.path().join("exports").join("opportunities.csv");
    export_opportunities_csv(&mut storage, opportunities.to_str().unwrap(), false).unwrap();
    let error =
        export_opportunities_csv(&mut storage, opportunities.to_str().unwrap(), false).unwrap_err();
    assert_eq!(error.kind(), "validation_failed");

    // Both exports are recorded in the command log.
    let logged: Vec<String> = {
        let mut statement = storage
            .connection()
            .prepare("SELECT entity_id FROM command_log WHERE entity_type='export' ORDER BY id")
            .unwrap();
        let rows = statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<String>>>()
            .unwrap();
        rows
    };
    assert_eq!(logged, ["contacts", "contacts", "opportunities"]);
}

#[test]
fn import_updates_are_patches_that_never_clear_unmapped_or_blank_fields() {
    let (temp, mut storage) = fixture();
    let first = write_csv(&temp, "first.csv", SAMPLE);
    let mapping = preview_contact_import(first.to_str().unwrap(), None)
        .unwrap()
        .mapping;
    import_contacts(
        &mut storage,
        ImportContactsRequest {
            actor: Actor::Import,
            path: first.to_str().unwrap().into(),
            mapping,
        },
    )
    .unwrap();
    let before = contact_by_display_name(&storage, "Dana Reyes");
    assert!(before.company_id.is_some());

    // A narrow file with only id, name, and phone must leave everything else
    // alone, including a blank cell in a mapped column.
    let narrow = write_csv(
        &temp,
        "narrow.csv",
        "External ID,Display Name,Phone Number,City\n\
         crm-1,Dana Reyes,555-0199,\n",
    );
    let mapping = preview_contact_import(narrow.to_str().unwrap(), None)
        .unwrap()
        .mapping;
    let summary = import_contacts(
        &mut storage,
        ImportContactsRequest {
            actor: Actor::Import,
            path: narrow.to_str().unwrap().into(),
            mapping,
        },
    )
    .unwrap();
    assert_eq!(summary.updated, 1);
    assert_eq!(summary.created, 0);

    let after = get_contact(&storage, &before.id).unwrap();
    assert_eq!(after.kind, before.kind);
    assert_eq!(after.company_id, before.company_id);
    assert_eq!(after.city, before.city); // blank cell kept "Ocala"
    assert_eq!(after.first_name, before.first_name);
    assert_eq!(after.last_name, before.last_name);
    assert_eq!(after.display_name, "Dana Reyes");
    assert_eq!(after.version, before.version + 1);
    // Channels are additive: the new phone joins the old ones.
    let mut values = after
        .channels
        .iter()
        .map(|channel| channel.value.clone())
        .collect::<Vec<_>>();
    values.sort();
    assert_eq!(values, ["555-0100", "555-0199", "dana@ridgeline.test"]);
    assert!(after
        .channels
        .iter()
        .any(|channel| channel.value == "555-0100" && channel.preferred));

    // Re-running the same file adds nothing new.
    let mapping = preview_contact_import(narrow.to_str().unwrap(), None)
        .unwrap()
        .mapping;
    import_contacts(
        &mut storage,
        ImportContactsRequest {
            actor: Actor::Import,
            path: narrow.to_str().unwrap().into(),
            mapping,
        },
    )
    .unwrap();
    assert_eq!(
        get_contact(&storage, &before.id).unwrap().channels.len(),
        after.channels.len()
    );
}

#[test]
fn import_skips_rows_matching_archived_contacts() {
    let (temp, mut storage) = fixture();
    let path = write_csv(&temp, "contacts.csv", SAMPLE);
    let mapping = preview_contact_import(path.to_str().unwrap(), None)
        .unwrap()
        .mapping;
    import_contacts(
        &mut storage,
        ImportContactsRequest {
            actor: Actor::Import,
            path: path.to_str().unwrap().into(),
            mapping: mapping.clone(),
        },
    )
    .unwrap();
    let dana = contact_by_display_name(&storage, "Dana Reyes");
    contractorcrm_lib::application::archive_contact(
        &mut storage,
        contractorcrm_lib::application::ArchiveRequest {
            actor: Actor::User,
            id: dana.id.clone(),
            expected_version: dana.version,
        },
    )
    .unwrap();

    let summary = import_contacts(
        &mut storage,
        ImportContactsRequest {
            actor: Actor::Import,
            path: path.to_str().unwrap().into(),
            mapping,
        },
    )
    .unwrap();
    assert_eq!(summary.created, 0);
    assert_eq!(summary.updated, 1); // the other row still updates
    assert_eq!(summary.skipped.len(), 1);
    assert_eq!(summary.skipped[0].line, 2);
    assert!(summary.skipped[0].reason.contains("archived contact"));
    assert!(summary.skipped[0].reason.contains(&dana.id));
    let unchanged = get_contact(&storage, &dana.id).unwrap();
    assert_eq!(unchanged.version, dana.version + 1); // archive only
}

#[test]
fn duplicate_and_interior_empty_headers_are_rejected() {
    let (temp, mut storage) = fixture();
    let duplicate = write_csv(
        &temp,
        "duplicate.csv",
        "Name,Email,Email\nDana Reyes,a@test,b@test\n",
    );
    let error = preview_contact_import(duplicate.to_str().unwrap(), None).unwrap_err();
    assert_eq!(error.kind(), "invalid_input");
    assert!(error.to_string().contains("duplicate column header"));
    let error = import_contacts(
        &mut storage,
        ImportContactsRequest {
            actor: Actor::Import,
            path: duplicate.to_str().unwrap().into(),
            mapping: ContactImportMapping {
                display_name: Some("Name".into()),
                ..Default::default()
            },
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), "invalid_input");

    let empty = write_csv(&temp, "empty.csv", "Name,,City\nDana Reyes,x,Ocala\n");
    let error = preview_contact_import(empty.to_str().unwrap(), None).unwrap_err();
    assert_eq!(error.kind(), "invalid_input");
    assert!(error.to_string().contains("empty header"));
    assert!(list_contacts(&storage, true).unwrap().is_empty());
}

#[test]
fn trailing_empty_header_columns_are_tolerated() {
    let (temp, mut storage) = fixture();
    // "Name,Email," is what a hand-edited spreadsheet export looks like.
    let path = write_csv(
        &temp,
        "trailing.csv",
        "Name,Email,\nDana Reyes,dana@ridgeline.test,\n",
    );
    let preview = preview_contact_import(path.to_str().unwrap(), None).unwrap();
    assert_eq!(preview.headers, ["Name", "Email"]);
    assert_eq!(preview.row_count, 1);
    assert!(preview.issues.is_empty());

    let summary = import_contacts(
        &mut storage,
        ImportContactsRequest {
            actor: Actor::Import,
            path: path.to_str().unwrap().into(),
            mapping: preview.mapping,
        },
    )
    .unwrap();
    assert_eq!(summary.created, 1);
    let dana = contact_by_display_name(&storage, "Dana Reyes");
    assert_eq!(dana.channels.len(), 1);
    assert_eq!(dana.channels[0].value, "dana@ridgeline.test");
}

#[test]
fn formula_guarded_exports_round_trip_byte_for_byte() {
    let (temp, mut storage) = fixture();
    let contact = create_contact(
        &mut storage,
        serde_json::from_value::<CreateContactRequest>(serde_json::json!({
            "displayName": "Dana Reyes",
            "kind": "client",
            "notes": "-leading dash note",
            "channels": [{"kind": "phone", "value": "+1 555 0100", "preferred": true}]
        }))
        .unwrap(),
    )
    .unwrap();

    let export_path = temp.path().join("contacts.csv");
    export_contacts_csv(&mut storage, export_path.to_str().unwrap(), false).unwrap();
    let exported = text(&export_path);
    assert!(exported.contains("'+1 555 0100")); // guarded on the way out
    assert!(exported.contains("'-leading dash note"));

    let mapping = preview_contact_import(export_path.to_str().unwrap(), None)
        .unwrap()
        .mapping;
    let summary = import_contacts(
        &mut storage,
        ImportContactsRequest {
            actor: Actor::Import,
            path: export_path.to_str().unwrap().into(),
            mapping,
        },
    )
    .unwrap();
    assert_eq!(summary.created, 0);
    assert_eq!(summary.updated, 1);

    // The guard quote is stripped on the way back in, so values are unchanged
    // and the existing phone is recognized instead of duplicated.
    let reloaded = get_contact(&storage, &contact.id).unwrap();
    assert_eq!(reloaded.notes.as_deref(), Some("-leading dash note"));
    assert_eq!(reloaded.channels.len(), 1);
    assert_eq!(reloaded.channels[0].value, "+1 555 0100");
}

#[test]
fn curated_display_names_survive_a_name_column_import() {
    let (temp, mut storage) = fixture();
    let curated = create_contact(
        &mut storage,
        serde_json::from_value::<CreateContactRequest>(serde_json::json!({
            "firstName": "Daniela",
            "lastName": "Reyes",
            "displayName": "Dana (site contact)",
            "kind": "client"
        }))
        .unwrap(),
    )
    .unwrap();
    let derived = create_contact(
        &mut storage,
        serde_json::from_value::<CreateContactRequest>(serde_json::json!({
            "firstName": "Sam",
            "lastName": "Ortiz",
            "kind": "client"
        }))
        .unwrap(),
    )
    .unwrap();

    let path = write_csv(
        &temp,
        "names.csv",
        &format!(
            "External ID,First Name,Last Name\n{},Daniela,Reyes-Kim\n{},Sam,Ortiz-Kim\n",
            curated.id, derived.id
        ),
    );
    let mapping = preview_contact_import(path.to_str().unwrap(), None)
        .unwrap()
        .mapping;
    let summary = import_contacts(
        &mut storage,
        ImportContactsRequest {
            actor: Actor::Import,
            path: path.to_str().unwrap().into(),
            mapping,
        },
    )
    .unwrap();
    assert_eq!(summary.updated, 2);

    // The curated display name is untouched; the auto-derived one follows the
    // new name parts.
    let curated = get_contact(&storage, &curated.id).unwrap();
    assert_eq!(curated.display_name, "Dana (site contact)");
    assert_eq!(curated.last_name.as_deref(), Some("Reyes-Kim"));
    let derived = get_contact(&storage, &derived.id).unwrap();
    assert_eq!(derived.display_name, "Sam Ortiz-Kim");
}

#[test]
fn non_utf8_files_report_an_encoding_error_not_a_storage_failure() {
    let (temp, _storage) = fixture();
    let path = temp.path().join("cp1252.csv");
    // "Se\xf1or" as Windows-1252 — invalid UTF-8.
    let mut bytes = b"Name,City\n".to_vec();
    bytes.extend_from_slice(&[0x53, 0x65, 0xf1, 0x6f, 0x72]);
    bytes.extend_from_slice(b",Ocala\n");
    std::fs::write(&path, bytes).unwrap();

    let error = preview_contact_import(path.to_str().unwrap(), None).unwrap_err();
    assert_eq!(error.kind(), "invalid_input");
    assert!(error.to_string().contains("UTF-8"));
}

#[test]
fn exports_neutralize_spreadsheet_formulas() {
    let (temp, mut storage) = fixture();
    create_contact(
        &mut storage,
        serde_json::from_value::<CreateContactRequest>(serde_json::json!({
            "displayName": "=HYPERLINK(\"http://evil.test\",\"click\")",
            "kind": "client",
            "notes": "+1 (555) 0100",
            "city": "@home"
        }))
        .unwrap(),
    )
    .unwrap();

    let export_path = temp.path().join("contacts.csv");
    export_contacts_csv(&mut storage, export_path.to_str().unwrap(), false).unwrap();
    let exported = text(&export_path);
    assert!(exported.contains("\"'=HYPERLINK("));
    assert!(exported.contains("'+1 (555) 0100"));
    assert!(exported.contains("'@home"));
    assert!(!exported.contains(",=HYPERLINK"));
}

#[test]
fn an_oversized_import_file_is_refused_before_it_is_buffered() {
    // The import is the one place a file nobody in this app wrote decides how
    // much memory the process spends, so the size is checked before the parse.
    let (temp, _storage) = fixture();
    let path = temp.path().join("huge.csv");
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(64 * 1024 * 1024 + 1).unwrap();
    drop(file);

    let error = preview_contact_import(path.to_str().unwrap(), None).unwrap_err();
    assert_eq!(error.kind(), "validation_failed");
    assert!(error.to_string().contains("limited to"), "{error}");
}

#[test]
fn an_import_file_with_too_many_rows_is_refused() {
    // A small file can still carry millions of rows; the row cap bounds that
    // without any of them reaching the database.
    let (temp, mut storage) = fixture();
    let mut body = String::from("External ID,Display Name\n");
    for index in 0..200_001 {
        body.push_str(&format!("crm-{index},Contact {index}\n"));
    }
    let path = write_csv(&temp, "many-rows.csv", &body);

    let error = preview_contact_import(path.to_str().unwrap(), None).unwrap_err();
    assert_eq!(error.kind(), "validation_failed");
    assert!(error.to_string().contains("rows"), "{error}");

    let error = import_contacts(
        &mut storage,
        ImportContactsRequest {
            actor: Actor::User,
            path: path.to_str().unwrap().to_owned(),
            mapping: ContactImportMapping::default(),
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), "validation_failed");
    assert!(list_contacts(&storage, false).unwrap().is_empty());
}
