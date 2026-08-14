//! Integration tests for the company/contact use-cases — happy paths,
//! validation failures, version conflicts, archive rules, channel
//! replacement, and command_log auditing.

use contractorcrm_lib::application::{
    archive_company, archive_contact, create_company, create_contact, get_company, get_contact,
    list_companies, list_contacts, unarchive_company, unarchive_contact, update_company,
    update_contact, ArchiveRequest, ChannelInput, CompanyPatch, ContactPatch, CreateCompanyRequest,
    CreateContactRequest, UpdateCompanyRequest, UpdateContactRequest,
};
use contractorcrm_lib::domain::{Actor, ChannelKind, Company, Contact, ContactRole, PartyKind};
use contractorcrm_lib::error::ApplicationError;
use contractorcrm_lib::storage::Storage;

fn open_storage(temp: &tempfile::TempDir) -> Storage {
    Storage::open_in_app_data(temp.path()).expect("open storage")
}

fn company_patch(name: &str, kind: &str) -> CompanyPatch {
    CompanyPatch {
        name: name.into(),
        kind: kind.into(),
        ..CompanyPatch::default()
    }
}

fn contact_patch(display_name: &str, kind: &str) -> ContactPatch {
    ContactPatch {
        display_name: Some(display_name.into()),
        kind: kind.into(),
        ..ContactPatch::default()
    }
}

fn phone(value: &str, preferred: bool) -> ChannelInput {
    ChannelInput {
        kind: "phone".into(),
        label: Some("mobile".into()),
        value: value.into(),
        preferred,
    }
}

fn make_company(storage: &mut Storage, name: &str, kind: &str) -> Company {
    create_company(
        storage,
        CreateCompanyRequest {
            actor: Actor::User,
            company: company_patch(name, kind),
        },
    )
    .expect("create company")
}

fn make_contact(storage: &mut Storage, patch: ContactPatch) -> Contact {
    create_contact(
        storage,
        CreateContactRequest {
            actor: Actor::User,
            contact: patch,
        },
    )
    .expect("create contact")
}

/// All command_log rows for one entity: (actor, entity_type, summary).
fn command_log_rows(storage: &Storage, entity_id: &str) -> Vec<(String, String, String)> {
    let mut statement = storage
        .connection()
        .prepare(
            "SELECT actor, entity_type, summary FROM command_log
             WHERE entity_id = ?1 ORDER BY created_at, id",
        )
        .expect("prepare command_log query");
    statement
        .query_map([entity_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .expect("query command_log")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect command_log rows")
}

// ---------------------------------------------------------------------------
// Companies
// ---------------------------------------------------------------------------

#[test]
fn create_company_stores_the_record_and_logs_the_command() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);

    let company = create_company(
        &mut storage,
        CreateCompanyRequest {
            actor: Actor::Agent,
            company: CompanyPatch {
                phone: Some("555-0100".into()),
                service_area: Some("Central Florida".into()),
                ..company_patch("Ridgeline Fence Co", "sub")
            },
        },
    )
    .expect("create company");

    assert_eq!(company.kind, PartyKind::Sub);
    assert_eq!(company.version, 1);
    assert!(company.archived_at.is_none());

    let fetched = get_company(&storage, &company.id).expect("get company");
    assert_eq!(fetched, company);

    assert_eq!(
        command_log_rows(&storage, &company.id),
        vec![(
            "agent".into(),
            "company".into(),
            "created company \"Ridgeline Fence Co\"".into()
        )]
    );
}

#[test]
fn create_company_rejects_empty_name_and_unknown_kind() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);

    let empty_name = create_company(
        &mut storage,
        CreateCompanyRequest {
            actor: Actor::User,
            company: company_patch("   ", "client"),
        },
    )
    .expect_err("empty name must fail");
    assert!(
        matches!(&empty_name, ApplicationError::InvalidInput { field, .. } if field == "name"),
        "unexpected error: {empty_name:?}"
    );

    let bad_kind = create_company(
        &mut storage,
        CreateCompanyRequest {
            actor: Actor::User,
            company: company_patch("Good Name", "franchise"),
        },
    )
    .expect_err("unknown kind must fail");
    assert_eq!(bad_kind.kind(), "invalid_input");
    assert!(
        matches!(&bad_kind, ApplicationError::InvalidInput { field, .. } if field == "kind"),
        "unexpected error: {bad_kind:?}"
    );
}

#[test]
fn update_company_bumps_version_and_rejects_stale_versions() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let company = make_company(&mut storage, "Ridgeline Fence Co", "sub");

    let updated = update_company(
        &mut storage,
        UpdateCompanyRequest {
            actor: Actor::User,
            company_id: company.id.clone(),
            expected_version: 1,
            patch: CompanyPatch {
                notes: Some("net-30 terms".into()),
                ..company_patch("Ridgeline Fence & Gate", "sub")
            },
        },
    )
    .expect("update company");
    assert_eq!(updated.name, "Ridgeline Fence & Gate");
    assert_eq!(updated.version, 2);
    assert!(updated.updated_at >= company.updated_at);

    // Stale expected version → version_conflict carrying the current version.
    let stale = update_company(
        &mut storage,
        UpdateCompanyRequest {
            actor: Actor::User,
            company_id: company.id.clone(),
            expected_version: 1,
            patch: company_patch("Too Late", "sub"),
        },
    )
    .expect_err("stale update must fail");
    assert_eq!(stale.kind(), "version_conflict");
    assert!(
        matches!(
            &stale,
            ApplicationError::VersionConflict {
                expected: 1,
                current: 2,
                ..
            }
        ),
        "unexpected error: {stale:?}"
    );

    // The stale update wrote nothing.
    assert_eq!(
        get_company(&storage, &company.id)
            .expect("get company")
            .name,
        "Ridgeline Fence & Gate"
    );
}

#[test]
fn archive_company_with_active_contacts_is_rejected() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let company = make_company(&mut storage, "Ridgeline Fence Co", "sub");
    let contact = make_contact(
        &mut storage,
        ContactPatch {
            company_id: Some(company.id.clone()),
            ..contact_patch("Sam Estimator", "sub")
        },
    );

    let rejected = archive_company(
        &mut storage,
        ArchiveRequest {
            actor: Actor::User,
            id: company.id.clone(),
            expected_version: 1,
        },
    )
    .expect_err("archive with active contact must fail");
    assert_eq!(rejected.kind(), "validation_failed");
    assert!(
        matches!(
            &rejected,
            ApplicationError::ValidationFailed { code, .. }
                if *code == "company_has_active_contacts"
        ),
        "unexpected error: {rejected:?}"
    );

    // Archive the contact first, then the company archives cleanly.
    archive_contact(
        &mut storage,
        ArchiveRequest {
            actor: Actor::User,
            id: contact.id.clone(),
            expected_version: 1,
        },
    )
    .expect("archive contact");
    let archived = archive_company(
        &mut storage,
        ArchiveRequest {
            actor: Actor::User,
            id: company.id.clone(),
            expected_version: 1,
        },
    )
    .expect("archive company");
    assert!(archived.archived_at.is_some());
    assert_eq!(archived.version, 2);
}

#[test]
fn company_archive_unarchive_round_trip_and_list_filtering() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let company = make_company(&mut storage, "Ridgeline Fence Co", "client");

    let archived = archive_company(
        &mut storage,
        ArchiveRequest {
            actor: Actor::User,
            id: company.id.clone(),
            expected_version: 1,
        },
    )
    .expect("archive company");
    assert!(archived.archived_at.is_some());

    // Default listing hides archived rows; include_archived shows them.
    assert!(list_companies(&storage, false)
        .expect("list active")
        .is_empty());
    assert_eq!(list_companies(&storage, true).expect("list all").len(), 1);

    let unarchived = unarchive_company(
        &mut storage,
        ArchiveRequest {
            actor: Actor::User,
            id: company.id.clone(),
            expected_version: 2,
        },
    )
    .expect("unarchive company");
    assert!(unarchived.archived_at.is_none());
    assert_eq!(unarchived.version, 3);
    assert_eq!(
        list_companies(&storage, false).expect("list active").len(),
        1
    );

    let summaries: Vec<String> = command_log_rows(&storage, &company.id)
        .into_iter()
        .map(|(_, _, summary)| summary)
        .collect();
    assert_eq!(
        summaries,
        vec![
            "created company \"Ridgeline Fence Co\"",
            "archived company \"Ridgeline Fence Co\"",
            "unarchived company \"Ridgeline Fence Co\"",
        ]
    );
}

#[test]
fn get_company_returns_not_found_for_unknown_ids() {
    let temp = tempfile::tempdir().expect("tempdir");
    let storage = open_storage(&temp);
    let missing = get_company(&storage, "no-such-id").expect_err("must be not found");
    assert_eq!(missing.kind(), "not_found");
}

// ---------------------------------------------------------------------------
// Contacts
// ---------------------------------------------------------------------------

#[test]
fn create_contact_stores_channels_and_derives_display_name() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);

    let contact = create_contact(
        &mut storage,
        CreateContactRequest {
            actor: Actor::User,
            contact: ContactPatch {
                first_name: Some("Dana".into()),
                last_name: Some("Homeowner".into()),
                display_name: None, // derived from first + last
                role: Some("owner".into()),
                channels: vec![
                    phone("555-0101", true),
                    ChannelInput {
                        kind: "email".into(),
                        label: None,
                        value: "dana@example.com".into(),
                        preferred: true,
                    },
                ],
                ..contact_patch("", "lead")
            },
        },
    )
    .expect("create contact");

    assert_eq!(contact.display_name, "Dana Homeowner");
    assert_eq!(contact.role, Some(ContactRole::Owner));
    assert_eq!(contact.kind, PartyKind::Lead);
    assert_eq!(contact.version, 1);
    assert_eq!(contact.channels.len(), 2);
    assert_eq!(contact.channels[0].kind, ChannelKind::Phone);
    assert_eq!(contact.channels[0].value, "555-0101");
    assert!(contact.channels[0].preferred);
    assert_eq!(contact.channels[0].sort_key, 0);
    assert_eq!(contact.channels[1].kind, ChannelKind::Email);
    assert_eq!(contact.channels[1].sort_key, 1);

    let fetched = get_contact(&storage, &contact.id).expect("get contact");
    assert_eq!(fetched, contact);
}

#[test]
fn create_contact_validation_failures() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);

    // No display name and no name parts.
    let nameless = create_contact(
        &mut storage,
        CreateContactRequest {
            actor: Actor::User,
            contact: ContactPatch {
                display_name: None,
                ..contact_patch("", "client")
            },
        },
    )
    .expect_err("nameless contact must fail");
    assert!(
        matches!(&nameless, ApplicationError::InvalidInput { field, .. } if field == "displayName"),
        "unexpected error: {nameless:?}"
    );

    // Unknown role enum value.
    let bad_role = create_contact(
        &mut storage,
        CreateContactRequest {
            actor: Actor::User,
            contact: ContactPatch {
                role: Some("foreman".into()),
                ..contact_patch("Sam", "client")
            },
        },
    )
    .expect_err("unknown role must fail");
    assert!(
        matches!(&bad_role, ApplicationError::InvalidInput { field, .. } if field == "role"),
        "unexpected error: {bad_role:?}"
    );

    // Empty channel value, with an indexed field path.
    let empty_value = create_contact(
        &mut storage,
        CreateContactRequest {
            actor: Actor::User,
            contact: ContactPatch {
                channels: vec![phone("  ", false)],
                ..contact_patch("Sam", "client")
            },
        },
    )
    .expect_err("empty channel value must fail");
    assert!(
        matches!(
            &empty_value,
            ApplicationError::InvalidInput { field, .. } if field == "channels[0].value"
        ),
        "unexpected error: {empty_value:?}"
    );

    // Two preferred phones — at most one preferred channel per kind.
    let two_preferred = create_contact(
        &mut storage,
        CreateContactRequest {
            actor: Actor::User,
            contact: ContactPatch {
                channels: vec![phone("555-0101", true), phone("555-0102", true)],
                ..contact_patch("Sam", "client")
            },
        },
    )
    .expect_err("two preferred phones must fail");
    assert_eq!(two_preferred.kind(), "validation_failed");
    assert!(
        matches!(
            &two_preferred,
            ApplicationError::ValidationFailed { code, field, .. }
                if *code == "duplicate_preferred_channel" && field == "channels[1].preferred"
        ),
        "unexpected error: {two_preferred:?}"
    );

    // Linking to a company that does not exist.
    let missing_company = create_contact(
        &mut storage,
        CreateContactRequest {
            actor: Actor::User,
            contact: ContactPatch {
                company_id: Some("no-such-company".into()),
                ..contact_patch("Sam", "client")
            },
        },
    )
    .expect_err("unknown company must fail");
    assert_eq!(missing_company.kind(), "not_found");

    // Nothing leaked into the tables from the failed attempts.
    assert!(list_contacts(&storage, true).expect("list").is_empty());
}

#[test]
fn update_contact_replaces_the_whole_channel_set() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(
        &mut storage,
        ContactPatch {
            channels: vec![phone("555-0101", true), phone("555-0102", false)],
            ..contact_patch("Dana Homeowner", "client")
        },
    );
    let old_channel_ids: Vec<String> = contact.channels.iter().map(|c| c.id.clone()).collect();

    let updated = update_contact(
        &mut storage,
        UpdateContactRequest {
            actor: Actor::Agent,
            contact_id: contact.id.clone(),
            expected_version: 1,
            patch: ContactPatch {
                channels: vec![ChannelInput {
                    kind: "email".into(),
                    label: Some("work".into()),
                    value: "dana@example.com".into(),
                    preferred: true,
                }],
                ..contact_patch("Dana Homeowner", "client")
            },
        },
    )
    .expect("update contact");

    assert_eq!(updated.version, 2);
    assert_eq!(updated.channels.len(), 1);
    assert_eq!(updated.channels[0].kind, ChannelKind::Email);
    assert!(!old_channel_ids.contains(&updated.channels[0].id));

    // Old channel rows are gone from the table, not just the projection.
    let remaining: i64 = storage
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM contact_channels WHERE contact_id = ?1",
            [&contact.id],
            |row| row.get(0),
        )
        .expect("count channels");
    assert_eq!(remaining, 1);

    assert_eq!(
        command_log_rows(&storage, &contact.id),
        vec![
            (
                "user".into(),
                "contact".into(),
                "created contact \"Dana Homeowner\"".into()
            ),
            (
                "agent".into(),
                "contact".into(),
                "updated contact \"Dana Homeowner\"".into()
            ),
        ]
    );
}

#[test]
fn update_contact_rejects_stale_versions() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, contact_patch("Dana Homeowner", "client"));

    update_contact(
        &mut storage,
        UpdateContactRequest {
            actor: Actor::User,
            contact_id: contact.id.clone(),
            expected_version: 1,
            patch: contact_patch("Dana H.", "client"),
        },
    )
    .expect("first update");

    let stale = update_contact(
        &mut storage,
        UpdateContactRequest {
            actor: Actor::User,
            contact_id: contact.id.clone(),
            expected_version: 1,
            patch: contact_patch("Too Late", "client"),
        },
    )
    .expect_err("stale update must fail");
    assert!(
        matches!(
            &stale,
            ApplicationError::VersionConflict {
                expected: 1,
                current: 2,
                ..
            }
        ),
        "unexpected error: {stale:?}"
    );
}

#[test]
fn contact_archive_unarchive_round_trip_and_list_filtering() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, contact_patch("Dana Homeowner", "client"));

    let archived = archive_contact(
        &mut storage,
        ArchiveRequest {
            actor: Actor::User,
            id: contact.id.clone(),
            expected_version: 1,
        },
    )
    .expect("archive contact");
    assert!(archived.archived_at.is_some());
    assert_eq!(archived.version, 2);
    assert!(list_contacts(&storage, false)
        .expect("list active")
        .is_empty());
    assert_eq!(list_contacts(&storage, true).expect("list all").len(), 1);

    let unarchived = unarchive_contact(
        &mut storage,
        ArchiveRequest {
            actor: Actor::User,
            id: contact.id.clone(),
            expected_version: 2,
        },
    )
    .expect("unarchive contact");
    assert!(unarchived.archived_at.is_none());
    assert_eq!(unarchived.version, 3);
    assert_eq!(
        list_contacts(&storage, false).expect("list active").len(),
        1
    );
}

#[test]
fn get_contact_returns_not_found_for_unknown_ids() {
    let temp = tempfile::tempdir().expect("tempdir");
    let storage = open_storage(&temp);
    let missing = get_contact(&storage, "no-such-id").expect_err("must be not found");
    assert_eq!(missing.kind(), "not_found");
}
