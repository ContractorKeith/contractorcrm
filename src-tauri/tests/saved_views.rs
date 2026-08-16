use contractorcrm_lib::application::{
    create_saved_view, delete_saved_view, list_saved_views, update_saved_view,
    CreateSavedViewRequest, DeleteSavedViewRequest, SavedViewDefinition, SavedViewEntityType,
    SavedViewFilter, SavedViewSort, SavedViewSortDirection, UpdateSavedViewRequest,
};
use contractorcrm_lib::domain::Actor;
use contractorcrm_lib::error::ApplicationError;
use contractorcrm_lib::storage::Storage;
use rusqlite::params;

fn definition(field: &str) -> SavedViewDefinition {
    SavedViewDefinition {
        schema_version: 1,
        filter: SavedViewFilter {
            include_archived: false,
        },
        sort: SavedViewSort {
            field: field.into(),
            direction: SavedViewSortDirection::Ascending,
        },
    }
}

fn create(
    storage: &mut Storage,
    name: &str,
    entity_type: SavedViewEntityType,
    field: &str,
) -> contractorcrm_lib::application::SavedView {
    create_saved_view(
        storage,
        CreateSavedViewRequest {
            actor: Actor::User,
            name: name.into(),
            entity_type,
            definition: definition(field),
        },
    )
    .expect("create saved view")
}

#[test]
fn saved_views_create_update_delete_reopen_and_keep_surface_order() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("crm.sqlite3");
    let mut storage = Storage::open(&path).expect("open");
    let first = create(
        &mut storage,
        "All contacts",
        SavedViewEntityType::Contact,
        "displayName",
    );
    let second = create(
        &mut storage,
        "All companies",
        SavedViewEntityType::Company,
        "name",
    );
    let updated = update_saved_view(
        &mut storage,
        UpdateSavedViewRequest {
            actor: Actor::Agent,
            saved_view_id: first.id.clone(),
            expected_version: 1,
            name: "Archived contacts".into(),
            definition: SavedViewDefinition {
                schema_version: 1,
                filter: SavedViewFilter {
                    include_archived: true,
                },
                sort: SavedViewSort {
                    field: "displayName".into(),
                    direction: SavedViewSortDirection::Descending,
                },
            },
        },
    )
    .expect("update");
    assert_eq!(updated.version, 2);
    assert!(updated.definition.filter.include_archived);
    assert_eq!(
        list_saved_views(&storage, SavedViewEntityType::Contact)
            .expect("list")
            .len(),
        1
    );
    assert_eq!(
        list_saved_views(&storage, SavedViewEntityType::Company).expect("list")[0].id,
        second.id
    );
    drop(storage);

    let mut reopened = Storage::open(&path).expect("reopen");
    let listed =
        list_saved_views(&reopened, SavedViewEntityType::Contact).expect("list after reopen");
    assert_eq!(listed, vec![updated.clone()]);
    delete_saved_view(
        &mut reopened,
        DeleteSavedViewRequest {
            actor: Actor::User,
            saved_view_id: updated.id,
            expected_version: 2,
        },
    )
    .expect("delete");
    assert!(list_saved_views(&reopened, SavedViewEntityType::Contact)
        .expect("list")
        .is_empty());
    let logged: i64 = reopened
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM command_log WHERE entity_type = 'saved_view'",
            [],
            |row| row.get(0),
        )
        .expect("saved-view command log");
    assert_eq!(logged, 4);
}

#[test]
fn saved_view_validation_conflicts_and_limits_are_honest() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = Storage::open_in_app_data(temp.path()).expect("open");
    let saved = create(&mut storage, "Open", SavedViewEntityType::Company, "name");
    let duplicate = create_saved_view(
        &mut storage,
        CreateSavedViewRequest {
            actor: Actor::User,
            name: "open".into(),
            entity_type: SavedViewEntityType::Company,
            definition: definition("name"),
        },
    )
    .expect_err("duplicate");
    assert!(matches!(
        duplicate,
        ApplicationError::ValidationFailed {
            code: "saved_view_name_taken",
            ..
        }
    ));
    create(
        &mut storage,
        "Open",
        SavedViewEntityType::Contact,
        "displayName",
    );
    let bad_sort = create_saved_view(
        &mut storage,
        CreateSavedViewRequest {
            actor: Actor::User,
            name: "Bad".into(),
            entity_type: SavedViewEntityType::Contact,
            definition: definition("value"),
        },
    )
    .expect_err("bad sort");
    assert!(matches!(
        bad_sort,
        ApplicationError::ValidationFailed {
            code: "invalid_saved_view_sort",
            ..
        }
    ));
    let stale = update_saved_view(
        &mut storage,
        UpdateSavedViewRequest {
            actor: Actor::User,
            saved_view_id: saved.id.clone(),
            expected_version: 9,
            name: "Later".into(),
            definition: definition("name"),
        },
    )
    .expect_err("stale");
    assert_eq!(stale.kind(), "version_conflict");
    let stale_delete = delete_saved_view(
        &mut storage,
        DeleteSavedViewRequest {
            actor: Actor::User,
            saved_view_id: saved.id,
            expected_version: 9,
        },
    )
    .expect_err("stale delete");
    assert_eq!(stale_delete.kind(), "version_conflict");
    for index in 0..49 {
        create(
            &mut storage,
            &format!("Company {index}"),
            SavedViewEntityType::Company,
            "name",
        );
    }
    let limit = create_saved_view(
        &mut storage,
        CreateSavedViewRequest {
            actor: Actor::User,
            name: "One too many".into(),
            entity_type: SavedViewEntityType::Company,
            definition: definition("name"),
        },
    )
    .expect_err("limit");
    assert!(matches!(
        limit,
        ApplicationError::ValidationFailed {
            code: "saved_view_limit_reached",
            ..
        }
    ));
}

#[test]
fn saved_views_accept_every_documented_sort_field() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = Storage::open_in_app_data(temp.path()).expect("open");
    for (entity_type, field) in [
        (SavedViewEntityType::Contact, "displayName"),
        (SavedViewEntityType::Company, "name"),
        (SavedViewEntityType::Opportunity, "name"),
        (SavedViewEntityType::Opportunity, "stage"),
        (SavedViewEntityType::Opportunity, "value"),
        (SavedViewEntityType::Opportunity, "expectedClose"),
    ] {
        let name = format!("{field} {:?}", entity_type);
        let view = create(&mut storage, &name, entity_type, field);
        assert_eq!(view.definition.sort.field, field);
    }
}

#[test]
fn stored_legacy_malformed_and_future_definitions_never_rewrite() {
    let temp = tempfile::tempdir().expect("tempdir");
    let storage = Storage::open_in_app_data(temp.path()).expect("open");
    let now = contractorcrm_lib::storage::now_utc();
    let legacy = r#"{"filter":{"includeArchived":false},"sort":{"field":"displayName","direction":"ascending"}}"#;
    storage.connection().execute("INSERT INTO saved_views (id,name,entity_type,definition_json,sort_key,created_at,updated_at,version) VALUES ('legacy','Legacy','contact',?1,0,?2,?2,1)", params![legacy, now]).expect("insert legacy");
    let listed = list_saved_views(&storage, SavedViewEntityType::Contact).expect("read legacy");
    assert_eq!(listed[0].definition.schema_version, 1);
    let stored: String = storage
        .connection()
        .query_row(
            "SELECT definition_json FROM saved_views WHERE id = 'legacy'",
            [],
            |row| row.get(0),
        )
        .expect("stored bytes");
    assert_eq!(stored, legacy);
    storage.connection().execute("INSERT INTO saved_views (id,name,entity_type,definition_json,sort_key,created_at,updated_at,version) VALUES ('future','Future','company','{\"schemaVersion\":2,\"filter\":{\"includeArchived\":false},\"sort\":{\"field\":\"name\",\"direction\":\"ascending\"}}',0,?1,?1,1)", [&now]).expect("insert future");
    let future =
        list_saved_views(&storage, SavedViewEntityType::Company).expect_err("future rejected");
    assert_eq!(future.kind(), "invalid_stored_data");
    let future_stored: String = storage
        .connection()
        .query_row(
            "SELECT definition_json FROM saved_views WHERE id = 'future'",
            [],
            |row| row.get(0),
        )
        .expect("future stored bytes");
    assert!(future_stored.contains("\"schemaVersion\":2"));
    storage
        .connection()
        .execute("DELETE FROM saved_views WHERE id = 'future'", [])
        .expect("remove future fixture");
    storage
        .connection()
        .execute(
            "INSERT INTO saved_views (id,name,entity_type,definition_json,sort_key,created_at,updated_at,version)
             VALUES ('malformed','Malformed','company','not-json',0,?1,?1,1)",
            [&now],
        )
        .expect("insert malformed");
    let malformed =
        list_saved_views(&storage, SavedViewEntityType::Company).expect_err("malformed rejected");
    assert_eq!(malformed.kind(), "invalid_stored_data");
    let malformed_stored: String = storage
        .connection()
        .query_row(
            "SELECT definition_json FROM saved_views WHERE id = 'malformed'",
            [],
            |row| row.get(0),
        )
        .expect("malformed stored bytes");
    assert_eq!(malformed_stored, "not-json");
}
