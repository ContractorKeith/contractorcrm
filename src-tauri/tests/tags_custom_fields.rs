use contractorcrm_lib::application::{
    archive_custom_field_def, archive_tag, create_company, create_custom_field_def, create_tag,
    get_record_metadata, list_custom_field_defs, list_tags, match_saved_view, set_record_metadata,
    unarchive_custom_field_def, unarchive_tag, update_custom_field_def, CreateCompanyRequest,
    CreateCustomFieldDefRequest, CreateTagRequest, CustomFieldDefArchiveRequest,
    CustomFieldOptionInput, CustomFieldValueInput, SavedViewCustomFieldPredicate,
    SavedViewDefinition, SavedViewEntityType, SavedViewFilter, SavedViewSort,
    SavedViewSortDirection, SetRecordMetadataRequest, TagArchiveRequest,
    UpdateCustomFieldDefRequest, UpdateTagRequest,
};
use contractorcrm_lib::domain::Actor;
use contractorcrm_lib::storage::Storage;

fn company(storage: &mut Storage) -> contractorcrm_lib::domain::Company {
    create_company(
        storage,
        CreateCompanyRequest {
            actor: Actor::User,
            company: Default::default(),
        },
    )
    .expect_err("company defaults invalid");
    // Use SQL-independent application input for the smallest valid owner.
    create_company(
        storage,
        serde_json::from_value(serde_json::json!({"name":"Acme","kind":"client"})).unwrap(),
    )
    .unwrap()
}

fn insert_owner(storage: &Storage, kind: &str, id: &str) {
    let now = contractorcrm_lib::storage::now_utc();
    match kind {
        "company" => storage.connection().execute("INSERT INTO companies (id,name,kind,created_at,updated_at,version) VALUES (?1,'Company','client',?2,?2,1)", rusqlite::params![id,now]).unwrap(),
        "contact" => storage.connection().execute("INSERT INTO contacts (id,display_name,kind,created_at,updated_at,version) VALUES (?1,'Contact','client',?2,?2,1)", rusqlite::params![id,now]).unwrap(),
        "opportunity" => storage.connection().execute("INSERT INTO opportunities (id,name,stage_id,value_minor,currency_code,created_at,updated_at,version) VALUES (?1,'Opportunity','stage-lead',0,'USD',?2,?2,1)", rusqlite::params![id,now]).unwrap(),
        _ => unreachable!(),
    };
}

fn definition(
    storage: &mut Storage,
    surface: SavedViewEntityType,
    label: &str,
    field_type: &str,
) -> contractorcrm_lib::application::CustomFieldDef {
    create_custom_field_def(
        storage,
        CreateCustomFieldDefRequest {
            actor: Actor::User,
            entity_type: surface,
            label: label.into(),
            field_type: field_type.into(),
            options: if field_type == "select" {
                vec![
                    CustomFieldOptionInput {
                        id: None,
                        label: "One".into(),
                    },
                    CustomFieldOptionInput {
                        id: None,
                        label: "Two".into(),
                    },
                ]
            } else {
                vec![]
            },
        },
    )
    .unwrap()
}

#[test]
fn metadata_replaces_atomically_and_identical_replay_is_a_noop() {
    let temp = tempfile::tempdir().unwrap();
    let mut storage = Storage::open_in_app_data(temp.path()).unwrap();
    let owner = company(&mut storage);
    let tag = create_tag(
        &mut storage,
        CreateTagRequest {
            actor: Actor::User,
            label: "Priority".into(),
            color_role: Some("attention".into()),
        },
    )
    .unwrap();
    let field = create_custom_field_def(
        &mut storage,
        CreateCustomFieldDefRequest {
            actor: Actor::User,
            entity_type: SavedViewEntityType::Company,
            label: "Type".into(),
            field_type: "select".into(),
            options: vec![CustomFieldOptionInput {
                id: None,
                label: "Residential".into(),
            }],
        },
    )
    .unwrap();
    let request = SetRecordMetadataRequest {
        actor: Actor::User,
        entity_type: SavedViewEntityType::Company,
        record_id: owner.id.clone(),
        expected_version: owner.version,
        tag_ids: vec![tag.id.clone()],
        values: vec![CustomFieldValueInput {
            definition_id: field.id.clone(),
            text_value: None,
            number_value: None,
            date_value: None,
            option_id: Some(field.options[0].id.clone()),
        }],
    };
    let metadata = set_record_metadata(&mut storage, request.clone()).unwrap();
    assert_eq!(metadata.tag_ids, vec![tag.id]);
    let version: i64 = storage
        .connection()
        .query_row(
            "SELECT version FROM companies WHERE id=?1",
            [&owner.id],
            |r| r.get(0),
        )
        .unwrap();
    let mut replay = request;
    replay.expected_version = version;
    set_record_metadata(&mut storage, replay).unwrap();
    let unchanged: i64 = storage
        .connection()
        .query_row(
            "SELECT version FROM companies WHERE id=?1",
            [&owner.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(version, unchanged);
    assert_eq!(
        get_record_metadata(&storage, SavedViewEntityType::Company, &owner.id)
            .unwrap()
            .values
            .len(),
        1
    );
}

#[test]
fn all_surfaces_and_typed_values_persist_across_restart() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("crm.sqlite3");
    let mut storage = Storage::open(&path).unwrap();
    insert_owner(&storage, "contact", "contact-metadata");
    insert_owner(&storage, "company", "company-metadata");
    insert_owner(&storage, "opportunity", "opportunity-metadata");
    let text = definition(&mut storage, SavedViewEntityType::Contact, "Text", "text");
    let number = definition(
        &mut storage,
        SavedViewEntityType::Company,
        "Number",
        "number",
    );
    let date = definition(
        &mut storage,
        SavedViewEntityType::Opportunity,
        "Date",
        "date",
    );
    for (surface, id, value) in [
        (
            SavedViewEntityType::Contact,
            "contact-metadata",
            CustomFieldValueInput {
                definition_id: text.id,
                text_value: Some("homeowner".into()),
                number_value: None,
                date_value: None,
                option_id: None,
            },
        ),
        (
            SavedViewEntityType::Company,
            "company-metadata",
            CustomFieldValueInput {
                definition_id: number.id,
                text_value: None,
                number_value: Some(12.5),
                date_value: None,
                option_id: None,
            },
        ),
        (
            SavedViewEntityType::Opportunity,
            "opportunity-metadata",
            CustomFieldValueInput {
                definition_id: date.id,
                text_value: None,
                number_value: None,
                date_value: Some("2026-08-17".into()),
                option_id: None,
            },
        ),
    ] {
        let version: i64 = storage
            .connection()
            .query_row(
                &format!(
                    "SELECT version FROM {} WHERE id=?1",
                    if surface == SavedViewEntityType::Contact {
                        "contacts"
                    } else if surface == SavedViewEntityType::Company {
                        "companies"
                    } else {
                        "opportunities"
                    }
                ),
                [id],
                |r| r.get(0),
            )
            .unwrap();
        set_record_metadata(
            &mut storage,
            SetRecordMetadataRequest {
                actor: Actor::User,
                entity_type: surface,
                record_id: id.into(),
                expected_version: version,
                tag_ids: vec![],
                values: vec![value],
            },
        )
        .unwrap();
    }
    drop(storage);
    let storage = Storage::open(&path).unwrap();
    assert_eq!(
        get_record_metadata(&storage, SavedViewEntityType::Contact, "contact-metadata")
            .unwrap()
            .values[0]
            .text_value
            .as_deref(),
        Some("homeowner")
    );
    assert_eq!(
        get_record_metadata(&storage, SavedViewEntityType::Company, "company-metadata")
            .unwrap()
            .values[0]
            .number_value,
        Some(12.5)
    );
    assert_eq!(
        get_record_metadata(
            &storage,
            SavedViewEntityType::Opportunity,
            "opportunity-metadata"
        )
        .unwrap()
        .values[0]
            .date_value
            .as_deref(),
        Some("2026-08-17")
    );
}

#[test]
fn lifecycle_validation_and_audit_failure_roll_back() {
    let temp = tempfile::tempdir().unwrap();
    let mut storage = Storage::open_in_app_data(temp.path()).unwrap();
    let owner = company(&mut storage);
    let tag = create_tag(
        &mut storage,
        CreateTagRequest {
            actor: Actor::User,
            label: "Archive me".into(),
            color_role: None,
        },
    )
    .unwrap();
    let field = definition(&mut storage, SavedViewEntityType::Company, "Pick", "select");
    archive_tag(
        &mut storage,
        TagArchiveRequest {
            actor: Actor::User,
            tag_id: tag.id.clone(),
            expected_version: 1,
        },
    )
    .unwrap();
    assert!(list_tags(&storage, false).unwrap().is_empty());
    unarchive_tag(
        &mut storage,
        TagArchiveRequest {
            actor: Actor::User,
            tag_id: tag.id.clone(),
            expected_version: 2,
        },
    )
    .unwrap();
    archive_custom_field_def(
        &mut storage,
        CustomFieldDefArchiveRequest {
            actor: Actor::User,
            definition_id: field.id.clone(),
            expected_version: 1,
        },
    )
    .unwrap();
    assert!(
        list_custom_field_defs(&storage, SavedViewEntityType::Company, false)
            .unwrap()
            .is_empty()
    );
    let field = unarchive_custom_field_def(
        &mut storage,
        CustomFieldDefArchiveRequest {
            actor: Actor::User,
            definition_id: field.id.clone(),
            expected_version: 2,
        },
    )
    .unwrap();
    let request = SetRecordMetadataRequest {
        actor: Actor::User,
        entity_type: SavedViewEntityType::Company,
        record_id: owner.id.clone(),
        expected_version: 1,
        tag_ids: vec![tag.id.clone()],
        values: vec![CustomFieldValueInput {
            definition_id: field.id.clone(),
            text_value: None,
            number_value: None,
            date_value: None,
            option_id: Some(field.options[0].id.clone()),
        }],
    };
    set_record_metadata(&mut storage, request).unwrap();
    let blocked = update_custom_field_def(
        &mut storage,
        UpdateCustomFieldDefRequest {
            actor: Actor::User,
            definition_id: field.id.clone(),
            expected_version: 3,
            label: "Pick".into(),
            sort_key: 0,
            options: vec![CustomFieldOptionInput {
                id: Some(field.options[1].id.clone()),
                label: "Two".into(),
            }],
        },
    )
    .unwrap_err();
    assert_eq!(blocked.kind(), "validation_failed");
    archive_tag(
        &mut storage,
        TagArchiveRequest {
            actor: Actor::User,
            tag_id: tag.id.clone(),
            expected_version: 3,
        },
    )
    .unwrap();
    archive_custom_field_def(
        &mut storage,
        CustomFieldDefArchiveRequest {
            actor: Actor::User,
            definition_id: field.id.clone(),
            expected_version: 3,
        },
    )
    .unwrap();
    let retained_value = get_record_metadata(&storage, SavedViewEntityType::Company, &owner.id)
        .unwrap()
        .values[0]
        .clone();
    set_record_metadata(
        &mut storage,
        SetRecordMetadataRequest {
            actor: Actor::User,
            entity_type: SavedViewEntityType::Company,
            record_id: owner.id.clone(),
            expected_version: 2,
            tag_ids: vec![],
            values: vec![CustomFieldValueInput {
                definition_id: retained_value.definition_id,
                text_value: retained_value.text_value,
                number_value: retained_value.number_value,
                date_value: retained_value.date_value,
                option_id: retained_value.option_id,
            }],
        },
    )
    .expect("archived metadata may be retained while another assignment is removed");
    storage.connection().execute("CREATE TRIGGER reject_metadata_audit BEFORE INSERT ON command_log WHEN NEW.summary='updated record metadata' BEGIN SELECT RAISE(ABORT,'audit failure'); END",[]).unwrap();
    let before: i64 = storage
        .connection()
        .query_row(
            "SELECT version FROM companies WHERE id=?1",
            [&owner.id],
            |r| r.get(0),
        )
        .unwrap();
    let failed = set_record_metadata(
        &mut storage,
        SetRecordMetadataRequest {
            actor: Actor::User,
            entity_type: SavedViewEntityType::Company,
            record_id: owner.id.clone(),
            expected_version: before,
            tag_ids: vec![],
            values: vec![],
        },
    )
    .unwrap_err();
    assert_eq!(failed.kind(), "storage_unavailable");
    let after: i64 = storage
        .connection()
        .query_row(
            "SELECT version FROM companies WHERE id=?1",
            [&owner.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(before, after);
    assert_eq!(
        get_record_metadata(&storage, SavedViewEntityType::Company, &owner.id)
            .unwrap()
            .values
            .len(),
        1
    );
    storage
        .connection()
        .execute("DROP TRIGGER reject_metadata_audit", [])
        .unwrap();
    set_record_metadata(
        &mut storage,
        SetRecordMetadataRequest {
            actor: Actor::User,
            entity_type: SavedViewEntityType::Company,
            record_id: owner.id.clone(),
            expected_version: before,
            tag_ids: vec![],
            values: vec![],
        },
    )
    .expect("archived custom-field values may be removed");
    assert!(
        get_record_metadata(&storage, SavedViewEntityType::Company, &owner.id)
            .unwrap()
            .values
            .is_empty()
    );
}

fn company_view(
    include_archived: bool,
    tags: Vec<String>,
    predicates: Vec<SavedViewCustomFieldPredicate>,
) -> SavedViewDefinition {
    SavedViewDefinition {
        schema_version: 2,
        filter: SavedViewFilter {
            include_archived,
            tag_ids_all: tags,
            custom_fields: predicates,
        },
        sort: SavedViewSort {
            field: "name".into(),
            direction: SavedViewSortDirection::Ascending,
        },
    }
}

#[test]
fn saved_view_matching_covers_tags_and_every_typed_operator() {
    let temp = tempfile::tempdir().unwrap();
    let mut storage = Storage::open_in_app_data(temp.path()).unwrap();
    insert_owner(&storage, "company", "company-match");
    insert_owner(&storage, "company", "company-other");
    let tag = create_tag(
        &mut storage,
        CreateTagRequest {
            actor: Actor::User,
            label: "Commercial".into(),
            color_role: Some("accent".into()),
        },
    )
    .unwrap();
    let text = definition(&mut storage, SavedViewEntityType::Company, "Notes", "text");
    let number = definition(&mut storage, SavedViewEntityType::Company, "Crew", "number");
    let date = definition(
        &mut storage,
        SavedViewEntityType::Company,
        "Renewal",
        "date",
    );
    let select = definition(&mut storage, SavedViewEntityType::Company, "Tier", "select");
    set_record_metadata(
        &mut storage,
        SetRecordMetadataRequest {
            actor: Actor::User,
            entity_type: SavedViewEntityType::Company,
            record_id: "company-match".into(),
            expected_version: 1,
            tag_ids: vec![tag.id.clone()],
            values: vec![
                CustomFieldValueInput {
                    definition_id: text.id.clone(),
                    text_value: Some("Preferred partner".into()),
                    number_value: None,
                    date_value: None,
                    option_id: None,
                },
                CustomFieldValueInput {
                    definition_id: number.id.clone(),
                    text_value: None,
                    number_value: Some(12.5),
                    date_value: None,
                    option_id: None,
                },
                CustomFieldValueInput {
                    definition_id: date.id.clone(),
                    text_value: None,
                    number_value: None,
                    date_value: Some("2026-09-01".into()),
                    option_id: None,
                },
                CustomFieldValueInput {
                    definition_id: select.id.clone(),
                    text_value: None,
                    number_value: None,
                    date_value: None,
                    option_id: Some(select.options[1].id.clone()),
                },
            ],
        },
    )
    .unwrap();

    for (definition_id, field_type, operator, value) in [
        (&text.id, "text", "contains", serde_json::json!("partner")),
        (
            &text.id,
            "text",
            "equals",
            serde_json::json!("Preferred partner"),
        ),
        (&number.id, "number", "equals", serde_json::json!(12.5)),
        (
            &number.id,
            "number",
            "greaterThanOrEqual",
            serde_json::json!(12),
        ),
        (
            &number.id,
            "number",
            "lessThanOrEqual",
            serde_json::json!(13),
        ),
        (&date.id, "date", "on", serde_json::json!("2026-09-01")),
        (&date.id, "date", "before", serde_json::json!("2026-10-01")),
        (&date.id, "date", "after", serde_json::json!("2026-08-01")),
        (
            &select.id,
            "select",
            "is",
            serde_json::json!(select.options[1].id),
        ),
    ] {
        let result = match_saved_view(
            &storage,
            SavedViewEntityType::Company,
            company_view(
                false,
                vec![tag.id.clone()],
                vec![SavedViewCustomFieldPredicate {
                    definition_id: definition_id.clone(),
                    field_type: field_type.into(),
                    operator: operator.into(),
                    value,
                }],
            ),
        )
        .unwrap();
        assert_eq!(result, vec!["company-match"]);
    }

    storage
        .connection()
        .execute(
            "UPDATE companies SET archived_at='2026-08-17T00:00:00.000Z' WHERE id='company-match'",
            [],
        )
        .unwrap();
    assert!(match_saved_view(
        &storage,
        SavedViewEntityType::Company,
        company_view(false, vec![tag.id.clone()], vec![])
    )
    .unwrap()
    .is_empty());
    assert_eq!(
        match_saved_view(
            &storage,
            SavedViewEntityType::Company,
            company_view(true, vec![tag.id], vec![])
        )
        .unwrap(),
        vec!["company-match"]
    );
}

#[test]
fn invalid_metadata_and_stale_saved_view_references_are_atomic() {
    let temp = tempfile::tempdir().unwrap();
    let mut storage = Storage::open_in_app_data(temp.path()).unwrap();
    insert_owner(&storage, "company", "company-invalid");
    let tag = create_tag(
        &mut storage,
        CreateTagRequest {
            actor: Actor::User,
            label: "Valid".into(),
            color_role: None,
        },
    )
    .unwrap();
    let text = definition(&mut storage, SavedViewEntityType::Company, "Text", "text");
    assert_eq!(
        create_tag(
            &mut storage,
            CreateTagRequest {
                actor: Actor::User,
                label: "valid".into(),
                color_role: None,
            },
        )
        .unwrap_err()
        .kind(),
        "validation_failed"
    );
    assert_eq!(
        contractorcrm_lib::application::update_tag(
            &mut storage,
            UpdateTagRequest {
                actor: Actor::User,
                tag_id: tag.id.clone(),
                expected_version: 9,
                label: "Later".into(),
                color_role: None,
            },
        )
        .unwrap_err()
        .kind(),
        "version_conflict"
    );
    assert_eq!(
        create_custom_field_def(
            &mut storage,
            CreateCustomFieldDefRequest {
                actor: Actor::User,
                entity_type: SavedViewEntityType::Company,
                label: "text".into(),
                field_type: "text".into(),
                options: vec![],
            },
        )
        .unwrap_err()
        .kind(),
        "validation_failed"
    );
    assert_eq!(
        update_custom_field_def(
            &mut storage,
            UpdateCustomFieldDefRequest {
                actor: Actor::User,
                definition_id: text.id.clone(),
                expected_version: 9,
                label: "Later".into(),
                sort_key: 0,
                options: vec![],
            },
        )
        .unwrap_err()
        .kind(),
        "version_conflict"
    );
    let date = definition(&mut storage, SavedViewEntityType::Company, "Date", "date");
    let select = definition(
        &mut storage,
        SavedViewEntityType::Company,
        "Select",
        "select",
    );
    let contact_text = definition(
        &mut storage,
        SavedViewEntityType::Contact,
        "Contact text",
        "text",
    );

    let invalid_values = vec![
        CustomFieldValueInput {
            definition_id: text.id.clone(),
            text_value: Some("x".into()),
            number_value: Some(1.0),
            date_value: None,
            option_id: None,
        },
        CustomFieldValueInput {
            definition_id: date.id.clone(),
            text_value: None,
            number_value: None,
            date_value: Some("08/17/2026".into()),
            option_id: None,
        },
        CustomFieldValueInput {
            definition_id: text.id.clone(),
            text_value: None,
            number_value: Some(f64::NAN),
            date_value: None,
            option_id: None,
        },
        CustomFieldValueInput {
            definition_id: select.id.clone(),
            text_value: None,
            number_value: None,
            date_value: None,
            option_id: Some("missing-option".into()),
        },
        CustomFieldValueInput {
            definition_id: contact_text.id,
            text_value: Some("wrong owner".into()),
            number_value: None,
            date_value: None,
            option_id: None,
        },
    ];
    for value in invalid_values {
        let error = set_record_metadata(
            &mut storage,
            SetRecordMetadataRequest {
                actor: Actor::User,
                entity_type: SavedViewEntityType::Company,
                record_id: "company-invalid".into(),
                expected_version: 1,
                tag_ids: vec![],
                values: vec![value],
            },
        )
        .unwrap_err();
        assert!(matches!(
            error.kind(),
            "invalid_input" | "validation_failed"
        ));
        assert!(
            get_record_metadata(&storage, SavedViewEntityType::Company, "company-invalid")
                .unwrap()
                .values
                .is_empty()
        );
    }
    let stale_version = set_record_metadata(
        &mut storage,
        SetRecordMetadataRequest {
            actor: Actor::User,
            entity_type: SavedViewEntityType::Company,
            record_id: "company-invalid".into(),
            expected_version: 9,
            tag_ids: vec![tag.id.clone()],
            values: vec![],
        },
    )
    .unwrap_err();
    assert_eq!(stale_version.kind(), "version_conflict");

    let archived = archive_tag(
        &mut storage,
        TagArchiveRequest {
            actor: Actor::User,
            tag_id: tag.id.clone(),
            expected_version: tag.version,
        },
    )
    .unwrap();
    let unavailable = set_record_metadata(
        &mut storage,
        SetRecordMetadataRequest {
            actor: Actor::User,
            entity_type: SavedViewEntityType::Company,
            record_id: "company-invalid".into(),
            expected_version: 1,
            tag_ids: vec![tag.id.clone()],
            values: vec![],
        },
    )
    .unwrap_err();
    assert_eq!(unavailable.kind(), "validation_failed");
    let stale_view = match_saved_view(
        &storage,
        SavedViewEntityType::Company,
        company_view(false, vec![archived.id], vec![]),
    )
    .unwrap_err();
    assert_eq!(stale_view.kind(), "validation_failed");
}
