//! Application use-cases for companies and contacts. Every mutation runs in
//! one immediate transaction, checks the expected record version, bumps the
//! version, and writes a command_log row before committing.

use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};

use crate::domain::{Actor, ChannelKind, Company, Contact, ContactChannel, ContactRole, PartyKind};
use crate::error::ApplicationError;
use crate::storage::{new_id, now_utc, Storage};

// ---------------------------------------------------------------------------
// Wire-shaped requests (camelCase)
// ---------------------------------------------------------------------------

/// Editable company fields; updates replace the full editable set (v1).
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompanyPatch {
    pub name: String,
    /// Wire enum value, e.g. "client"; validated by the application layer.
    pub kind: String,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub website: Option<String>,
    pub address_line1: Option<String>,
    pub address_line2: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub postal_code: Option<String>,
    pub service_area: Option<String>,
    pub license_notes: Option<String>,
    pub notes: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCompanyRequest {
    #[serde(default)]
    pub actor: Actor,
    #[serde(flatten)]
    pub company: CompanyPatch,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCompanyRequest {
    #[serde(default)]
    pub actor: Actor,
    pub company_id: String,
    pub expected_version: i64,
    pub patch: CompanyPatch,
}

/// One phone or email in a create/update request; ids are assigned on write.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelInput {
    /// Wire enum value, "phone" or "email"; validated by the application layer.
    pub kind: String,
    pub label: Option<String>,
    pub value: String,
    #[serde(default)]
    pub preferred: bool,
}

/// Editable contact fields; updates replace the full editable set including
/// the whole channel list (simplest correct v1).
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactPatch {
    pub company_id: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    /// Derived from first/last names when absent.
    pub display_name: Option<String>,
    /// Wire enum value, e.g. "estimator"; optional.
    pub role: Option<String>,
    /// Wire enum value, e.g. "client"; validated by the application layer.
    pub kind: String,
    pub preferred_contact_method: Option<String>,
    pub address_line1: Option<String>,
    pub address_line2: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub postal_code: Option<String>,
    pub property_type: Option<String>,
    pub notes: Option<String>,
    #[serde(default)]
    pub favorite: bool,
    #[serde(default)]
    pub channels: Vec<ChannelInput>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateContactRequest {
    #[serde(default)]
    pub actor: Actor,
    #[serde(flatten)]
    pub contact: ContactPatch,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateContactRequest {
    #[serde(default)]
    pub actor: Actor,
    pub contact_id: String,
    pub expected_version: i64,
    pub patch: ContactPatch,
}

/// Shared shape for archive/unarchive of either record type.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveRequest {
    #[serde(default)]
    pub actor: Actor,
    pub id: String,
    pub expected_version: i64,
}

// ---------------------------------------------------------------------------
// Company use-cases
// ---------------------------------------------------------------------------

/// Create a company and log the command; returns the stored record.
pub fn create_company(
    storage: &mut Storage,
    request: CreateCompanyRequest,
) -> Result<Company, ApplicationError> {
    let fields = validate_company_patch(request.company)?;
    let now = now_utc();
    let company = Company {
        id: new_id(),
        name: fields.name,
        kind: fields.kind,
        phone: fields.phone,
        email: fields.email,
        website: fields.website,
        address_line1: fields.address_line1,
        address_line2: fields.address_line2,
        city: fields.city,
        state: fields.state,
        postal_code: fields.postal_code,
        service_area: fields.service_area,
        license_notes: fields.license_notes,
        notes: fields.notes,
        archived_at: None,
        created_at: now.clone(),
        updated_at: now,
        version: 1,
    };

    let transaction = immediate(storage)?;
    transaction.execute(
        "INSERT INTO companies (
            id, name, kind, phone, email, website,
            address_line1, address_line2, city, state, postal_code,
            service_area, license_notes, notes,
            archived_at, created_at, updated_at, version
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                   ?15, ?16, ?17, ?18)",
        params![
            company.id,
            company.name,
            company.kind.as_database_value(),
            company.phone,
            company.email,
            company.website,
            company.address_line1,
            company.address_line2,
            company.city,
            company.state,
            company.postal_code,
            company.service_area,
            company.license_notes,
            company.notes,
            company.archived_at,
            company.created_at,
            company.updated_at,
            company.version,
        ],
    )?;
    log_command(
        &transaction,
        request.actor,
        "company",
        &company.id,
        &format!("created company \"{}\"", company.name),
    )?;
    transaction.commit()?;
    Ok(company)
}

/// Replace a company's editable fields; requires the expected version.
pub fn update_company(
    storage: &mut Storage,
    request: UpdateCompanyRequest,
) -> Result<Company, ApplicationError> {
    let fields = validate_company_patch(request.patch)?;
    let transaction = immediate(storage)?;
    let mut company = require_company(&transaction, &request.company_id)?;
    check_version(
        "company",
        &company.id,
        request.expected_version,
        company.version,
    )?;

    company.name = fields.name;
    company.kind = fields.kind;
    company.phone = fields.phone;
    company.email = fields.email;
    company.website = fields.website;
    company.address_line1 = fields.address_line1;
    company.address_line2 = fields.address_line2;
    company.city = fields.city;
    company.state = fields.state;
    company.postal_code = fields.postal_code;
    company.service_area = fields.service_area;
    company.license_notes = fields.license_notes;
    company.notes = fields.notes;
    company.updated_at = now_utc();
    company.version += 1;

    transaction.execute(
        "UPDATE companies SET
            name = ?2, kind = ?3, phone = ?4, email = ?5, website = ?6,
            address_line1 = ?7, address_line2 = ?8, city = ?9, state = ?10,
            postal_code = ?11, service_area = ?12, license_notes = ?13,
            notes = ?14, updated_at = ?15, version = ?16
         WHERE id = ?1",
        params![
            company.id,
            company.name,
            company.kind.as_database_value(),
            company.phone,
            company.email,
            company.website,
            company.address_line1,
            company.address_line2,
            company.city,
            company.state,
            company.postal_code,
            company.service_area,
            company.license_notes,
            company.notes,
            company.updated_at,
            company.version,
        ],
    )?;
    log_command(
        &transaction,
        request.actor,
        "company",
        &company.id,
        &format!("updated company \"{}\"", company.name),
    )?;
    transaction.commit()?;
    Ok(company)
}

/// Archive a company; rejected while it still has active contacts (no
/// cascading archive in v1).
pub fn archive_company(
    storage: &mut Storage,
    request: ArchiveRequest,
) -> Result<Company, ApplicationError> {
    set_company_archived(storage, request, true)
}

/// Bring an archived company back.
pub fn unarchive_company(
    storage: &mut Storage,
    request: ArchiveRequest,
) -> Result<Company, ApplicationError> {
    set_company_archived(storage, request, false)
}

fn set_company_archived(
    storage: &mut Storage,
    request: ArchiveRequest,
    archived: bool,
) -> Result<Company, ApplicationError> {
    let transaction = immediate(storage)?;
    let mut company = require_company(&transaction, &request.id)?;
    check_version(
        "company",
        &company.id,
        request.expected_version,
        company.version,
    )?;

    if archived {
        let active_contacts: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM contacts WHERE company_id = ?1 AND archived_at IS NULL",
            [&company.id],
            |row| row.get(0),
        )?;
        if active_contacts > 0 {
            return Err(ApplicationError::ValidationFailed {
                code: "company_has_active_contacts",
                field: "companyId".into(),
                message: format!(
                    "cannot archive company \"{}\": it still has {active_contacts} active \
                     contact(s); archive or reassign them first",
                    company.name
                ),
            });
        }
    }

    company.archived_at = archived.then(now_utc);
    company.updated_at = now_utc();
    company.version += 1;
    transaction.execute(
        "UPDATE companies SET archived_at = ?2, updated_at = ?3, version = ?4 WHERE id = ?1",
        params![
            company.id,
            company.archived_at,
            company.updated_at,
            company.version,
        ],
    )?;
    let verb = if archived { "archived" } else { "unarchived" };
    log_command(
        &transaction,
        request.actor,
        "company",
        &company.id,
        &format!("{verb} company \"{}\"", company.name),
    )?;
    transaction.commit()?;
    Ok(company)
}

/// List companies by name; archived rows only when asked for.
pub fn list_companies(
    storage: &Storage,
    include_archived: bool,
) -> Result<Vec<Company>, ApplicationError> {
    let mut statement = storage.connection().prepare(&format!(
        "SELECT {COMPANY_COLUMNS} FROM companies {} ORDER BY name, id",
        if include_archived {
            ""
        } else {
            "WHERE archived_at IS NULL"
        }
    ))?;
    let companies = statement
        .query_map([], company_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    companies.into_iter().map(finish_company).collect()
}

/// Fetch one company by id, archived or not.
pub fn get_company(storage: &Storage, company_id: &str) -> Result<Company, ApplicationError> {
    let row = storage
        .connection()
        .query_row(
            &format!("SELECT {COMPANY_COLUMNS} FROM companies WHERE id = ?1"),
            [company_id],
            company_from_row,
        )
        .optional()?
        .ok_or_else(|| ApplicationError::NotFound {
            resource: "company",
            id: company_id.into(),
        })?;
    finish_company(row)
}

// ---------------------------------------------------------------------------
// Contact use-cases
// ---------------------------------------------------------------------------

/// Create a contact plus its channels and log the command.
pub fn create_contact(
    storage: &mut Storage,
    request: CreateContactRequest,
) -> Result<Contact, ApplicationError> {
    let fields = validate_contact_patch(request.contact)?;
    let now = now_utc();
    let contact_id = new_id();

    let transaction = immediate(storage)?;
    require_linked_company(&transaction, fields.company_id.as_deref())?;
    transaction.execute(
        "INSERT INTO contacts (
            id, company_id, first_name, last_name, display_name, role, kind,
            preferred_contact_method, address_line1, address_line2, city, state,
            postal_code, property_type, notes, favorite,
            archived_at, created_at, updated_at, version
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                   ?15, ?16, NULL, ?17, ?17, 1)",
        params![
            contact_id,
            fields.company_id,
            fields.first_name,
            fields.last_name,
            fields.display_name,
            fields.role.map(ContactRole::as_database_value),
            fields.kind.as_database_value(),
            fields.preferred_contact_method,
            fields.address_line1,
            fields.address_line2,
            fields.city,
            fields.state,
            fields.postal_code,
            fields.property_type,
            fields.notes,
            fields.favorite,
            now,
        ],
    )?;
    insert_channels(&transaction, &contact_id, &fields.channels)?;
    log_command(
        &transaction,
        request.actor,
        "contact",
        &contact_id,
        &format!("created contact \"{}\"", fields.display_name),
    )?;
    let contact = require_contact(&transaction, &contact_id)?;
    transaction.commit()?;
    Ok(contact)
}

/// Replace a contact's editable fields and its whole channel set atomically;
/// requires the expected version.
pub fn update_contact(
    storage: &mut Storage,
    request: UpdateContactRequest,
) -> Result<Contact, ApplicationError> {
    let fields = validate_contact_patch(request.patch)?;
    let transaction = immediate(storage)?;
    let existing = require_contact(&transaction, &request.contact_id)?;
    check_version(
        "contact",
        &existing.id,
        request.expected_version,
        existing.version,
    )?;
    require_linked_company(&transaction, fields.company_id.as_deref())?;

    transaction.execute(
        "UPDATE contacts SET
            company_id = ?2, first_name = ?3, last_name = ?4, display_name = ?5,
            role = ?6, kind = ?7, preferred_contact_method = ?8,
            address_line1 = ?9, address_line2 = ?10, city = ?11, state = ?12,
            postal_code = ?13, property_type = ?14, notes = ?15, favorite = ?16,
            updated_at = ?17, version = ?18
         WHERE id = ?1",
        params![
            existing.id,
            fields.company_id,
            fields.first_name,
            fields.last_name,
            fields.display_name,
            fields.role.map(ContactRole::as_database_value),
            fields.kind.as_database_value(),
            fields.preferred_contact_method,
            fields.address_line1,
            fields.address_line2,
            fields.city,
            fields.state,
            fields.postal_code,
            fields.property_type,
            fields.notes,
            fields.favorite,
            now_utc(),
            existing.version + 1,
        ],
    )?;
    // Full channel replacement — delete then re-insert inside the transaction.
    transaction.execute(
        "DELETE FROM contact_channels WHERE contact_id = ?1",
        [&existing.id],
    )?;
    insert_channels(&transaction, &existing.id, &fields.channels)?;
    log_command(
        &transaction,
        request.actor,
        "contact",
        &existing.id,
        &format!("updated contact \"{}\"", fields.display_name),
    )?;
    let contact = require_contact(&transaction, &existing.id)?;
    transaction.commit()?;
    Ok(contact)
}

/// Archive a contact; history stays, the record leaves default lists.
pub fn archive_contact(
    storage: &mut Storage,
    request: ArchiveRequest,
) -> Result<Contact, ApplicationError> {
    set_contact_archived(storage, request, true)
}

/// Bring an archived contact back.
pub fn unarchive_contact(
    storage: &mut Storage,
    request: ArchiveRequest,
) -> Result<Contact, ApplicationError> {
    set_contact_archived(storage, request, false)
}

fn set_contact_archived(
    storage: &mut Storage,
    request: ArchiveRequest,
    archived: bool,
) -> Result<Contact, ApplicationError> {
    let transaction = immediate(storage)?;
    let mut contact = require_contact(&transaction, &request.id)?;
    check_version(
        "contact",
        &contact.id,
        request.expected_version,
        contact.version,
    )?;

    contact.archived_at = archived.then(now_utc);
    contact.updated_at = now_utc();
    contact.version += 1;
    transaction.execute(
        "UPDATE contacts SET archived_at = ?2, updated_at = ?3, version = ?4 WHERE id = ?1",
        params![
            contact.id,
            contact.archived_at,
            contact.updated_at,
            contact.version,
        ],
    )?;
    let verb = if archived { "archived" } else { "unarchived" };
    log_command(
        &transaction,
        request.actor,
        "contact",
        &contact.id,
        &format!("{verb} contact \"{}\"", contact.display_name),
    )?;
    transaction.commit()?;
    Ok(contact)
}

/// List contacts with their channels; archived rows only when asked for.
pub fn list_contacts(
    storage: &Storage,
    include_archived: bool,
) -> Result<Vec<Contact>, ApplicationError> {
    let connection = storage.connection();
    let mut statement = connection.prepare(&format!(
        "SELECT {CONTACT_COLUMNS} FROM contacts {} ORDER BY display_name, id",
        if include_archived {
            ""
        } else {
            "WHERE archived_at IS NULL"
        }
    ))?;
    let rows = statement
        .query_map([], contact_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|row| {
            let mut contact = finish_contact(row)?;
            contact.channels = load_channels(connection, &contact.id)?;
            Ok(contact)
        })
        .collect()
}

/// Fetch one contact by id with channels, archived or not.
pub fn get_contact(storage: &Storage, contact_id: &str) -> Result<Contact, ApplicationError> {
    let connection = storage.connection();
    let row = connection
        .query_row(
            &format!("SELECT {CONTACT_COLUMNS} FROM contacts WHERE id = ?1"),
            [contact_id],
            contact_from_row,
        )
        .optional()?
        .ok_or_else(|| ApplicationError::NotFound {
            resource: "contact",
            id: contact_id.into(),
        })?;
    let mut contact = finish_contact(row)?;
    contact.channels = load_channels(connection, &contact.id)?;
    Ok(contact)
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Company patch after validation, with parsed enums and trimmed text.
struct ValidCompanyFields {
    name: String,
    kind: PartyKind,
    phone: Option<String>,
    email: Option<String>,
    website: Option<String>,
    address_line1: Option<String>,
    address_line2: Option<String>,
    city: Option<String>,
    state: Option<String>,
    postal_code: Option<String>,
    service_area: Option<String>,
    license_notes: Option<String>,
    notes: Option<String>,
}

fn validate_company_patch(patch: CompanyPatch) -> Result<ValidCompanyFields, ApplicationError> {
    Ok(ValidCompanyFields {
        name: required_text("name", patch.name, 200)?,
        kind: parse_party_kind("kind", &patch.kind)?,
        phone: optional_text(patch.phone),
        email: optional_text(patch.email),
        website: optional_text(patch.website),
        address_line1: optional_text(patch.address_line1),
        address_line2: optional_text(patch.address_line2),
        city: optional_text(patch.city),
        state: optional_text(patch.state),
        postal_code: optional_text(patch.postal_code),
        service_area: optional_text(patch.service_area),
        license_notes: optional_text(patch.license_notes),
        notes: optional_text(patch.notes),
    })
}

/// Channel after validation, ready to insert.
struct ValidChannel {
    kind: ChannelKind,
    label: Option<String>,
    value: String,
    preferred: bool,
}

/// Contact patch after validation, with parsed enums and a derived display name.
struct ValidContactFields {
    company_id: Option<String>,
    first_name: Option<String>,
    last_name: Option<String>,
    display_name: String,
    role: Option<ContactRole>,
    kind: PartyKind,
    preferred_contact_method: Option<String>,
    address_line1: Option<String>,
    address_line2: Option<String>,
    city: Option<String>,
    state: Option<String>,
    postal_code: Option<String>,
    property_type: Option<String>,
    notes: Option<String>,
    favorite: bool,
    channels: Vec<ValidChannel>,
}

fn validate_contact_patch(patch: ContactPatch) -> Result<ValidContactFields, ApplicationError> {
    let first_name = optional_text(patch.first_name);
    let last_name = optional_text(patch.last_name);
    let display_name =
        derive_display_name(optional_text(patch.display_name), &first_name, &last_name)?;
    let role = match optional_text(patch.role) {
        None => None,
        Some(value) => Some(ContactRole::from_database_value(&value).ok_or_else(|| {
            ApplicationError::InvalidInput {
                field: "role".into(),
                message: format!(
                    "unknown role \"{value}\"; expected one of owner, estimator, \
                     site_contact, office, other"
                ),
            }
        })?),
    };
    Ok(ValidContactFields {
        company_id: optional_text(patch.company_id),
        first_name,
        last_name,
        display_name,
        role,
        kind: parse_party_kind("kind", &patch.kind)?,
        preferred_contact_method: optional_text(patch.preferred_contact_method),
        address_line1: optional_text(patch.address_line1),
        address_line2: optional_text(patch.address_line2),
        city: optional_text(patch.city),
        state: optional_text(patch.state),
        postal_code: optional_text(patch.postal_code),
        property_type: optional_text(patch.property_type),
        notes: optional_text(patch.notes),
        favorite: patch.favorite,
        channels: validate_channels(patch.channels)?,
    })
}

/// Explicit display name wins; otherwise derive "First Last" from the parts.
fn derive_display_name(
    display_name: Option<String>,
    first_name: &Option<String>,
    last_name: &Option<String>,
) -> Result<String, ApplicationError> {
    if let Some(name) = display_name {
        return Ok(name);
    }
    let derived = [first_name.as_deref(), last_name.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
    if derived.is_empty() {
        return Err(ApplicationError::InvalidInput {
            field: "displayName".into(),
            message: "is required when first and last name are both empty".into(),
        });
    }
    Ok(derived)
}

/// Channel rules: known kind, non-empty value, at most one preferred per kind.
fn validate_channels(inputs: Vec<ChannelInput>) -> Result<Vec<ValidChannel>, ApplicationError> {
    let mut preferred_kinds: Vec<ChannelKind> = Vec::new();
    let mut channels = Vec::with_capacity(inputs.len());
    for (index, input) in inputs.into_iter().enumerate() {
        let kind = ChannelKind::from_database_value(input.kind.trim()).ok_or_else(|| {
            ApplicationError::InvalidInput {
                field: format!("channels[{index}].kind"),
                message: format!(
                    "unknown channel kind \"{}\"; expected phone or email",
                    input.kind
                ),
            }
        })?;
        let value = required_text(format!("channels[{index}].value"), input.value, 200)?;
        if input.preferred {
            if preferred_kinds.contains(&kind) {
                return Err(ApplicationError::ValidationFailed {
                    code: "duplicate_preferred_channel",
                    field: format!("channels[{index}].preferred"),
                    message: format!(
                        "at most one preferred {} is allowed per contact",
                        kind.as_database_value()
                    ),
                });
            }
            preferred_kinds.push(kind);
        }
        channels.push(ValidChannel {
            kind,
            label: optional_text(input.label),
            value,
            preferred: input.preferred,
        });
    }
    Ok(channels)
}

fn parse_party_kind(field: &str, value: &str) -> Result<PartyKind, ApplicationError> {
    PartyKind::from_database_value(value.trim()).ok_or_else(|| ApplicationError::InvalidInput {
        field: field.into(),
        message: format!(
            "unknown kind \"{value}\"; expected one of client, lead, sub, vendor, \
             supplier, other"
        ),
    })
}

fn required_text(
    field: impl Into<String>,
    value: String,
    maximum_characters: usize,
) -> Result<String, ApplicationError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ApplicationError::InvalidInput {
            field: field.into(),
            message: "is required".into(),
        });
    }
    if value.chars().count() > maximum_characters {
        return Err(ApplicationError::InvalidInput {
            field: field.into(),
            message: format!("must be {maximum_characters} characters or fewer"),
        });
    }
    Ok(value.into())
}

/// Trim optional text; blank collapses to NULL.
fn optional_text(value: Option<String>) -> Option<String> {
    let value = value?.trim().to_string();
    (!value.is_empty()).then_some(value)
}

// ---------------------------------------------------------------------------
// Repository helpers (SQL on the open transaction/connection)
// ---------------------------------------------------------------------------

const COMPANY_COLUMNS: &str = "id, name, kind, phone, email, website, \
    address_line1, address_line2, city, state, postal_code, service_area, \
    license_notes, notes, archived_at, created_at, updated_at, version";

const CONTACT_COLUMNS: &str = "id, company_id, first_name, last_name, display_name, \
    role, kind, preferred_contact_method, address_line1, address_line2, city, state, \
    postal_code, property_type, notes, favorite, archived_at, created_at, updated_at, version";

fn immediate(storage: &mut Storage) -> Result<Transaction<'_>, ApplicationError> {
    Ok(storage
        .connection_mut()
        .transaction_with_behavior(TransactionBehavior::Immediate)?)
}

fn check_version(
    resource: &'static str,
    id: &str,
    expected: i64,
    current: i64,
) -> Result<(), ApplicationError> {
    if expected != current {
        return Err(ApplicationError::VersionConflict {
            resource,
            id: id.into(),
            expected,
            current,
        });
    }
    Ok(())
}

/// Row tuple with the raw kind text, finished into a Company after parsing.
type CompanyRow = (Company, String);

fn company_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CompanyRow> {
    let kind_text: String = row.get(2)?;
    Ok((
        Company {
            id: row.get(0)?,
            name: row.get(1)?,
            kind: PartyKind::Other, // replaced in finish_company
            phone: row.get(3)?,
            email: row.get(4)?,
            website: row.get(5)?,
            address_line1: row.get(6)?,
            address_line2: row.get(7)?,
            city: row.get(8)?,
            state: row.get(9)?,
            postal_code: row.get(10)?,
            service_area: row.get(11)?,
            license_notes: row.get(12)?,
            notes: row.get(13)?,
            archived_at: row.get(14)?,
            created_at: row.get(15)?,
            updated_at: row.get(16)?,
            version: row.get(17)?,
        },
        kind_text,
    ))
}

/// Parse the stored kind text once the rusqlite row mapping is done.
fn finish_company((mut company, kind_text): CompanyRow) -> Result<Company, ApplicationError> {
    company.kind = PartyKind::from_database_value(&kind_text).ok_or_else(|| {
        ApplicationError::InvalidStoredData(format!(
            "company {} has unsupported kind {kind_text}",
            company.id
        ))
    })?;
    Ok(company)
}

fn require_company(
    transaction: &Transaction<'_>,
    company_id: &str,
) -> Result<Company, ApplicationError> {
    let row = transaction
        .query_row(
            &format!("SELECT {COMPANY_COLUMNS} FROM companies WHERE id = ?1"),
            [company_id],
            company_from_row,
        )
        .optional()?
        .ok_or_else(|| ApplicationError::NotFound {
            resource: "company",
            id: company_id.into(),
        })?;
    finish_company(row)
}

/// A contact's linked company must exist (any archive state).
fn require_linked_company(
    transaction: &Transaction<'_>,
    company_id: Option<&str>,
) -> Result<(), ApplicationError> {
    let Some(company_id) = company_id else {
        return Ok(());
    };
    let exists: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM companies WHERE id = ?1)",
        [company_id],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(ApplicationError::NotFound {
            resource: "company",
            id: company_id.into(),
        });
    }
    Ok(())
}

/// Row tuple with raw enum text, finished into a Contact after parsing.
type ContactRow = (Contact, String, Option<String>);

fn contact_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ContactRow> {
    let role_text: Option<String> = row.get(5)?;
    let kind_text: String = row.get(6)?;
    Ok((
        Contact {
            id: row.get(0)?,
            company_id: row.get(1)?,
            first_name: row.get(2)?,
            last_name: row.get(3)?,
            display_name: row.get(4)?,
            role: None,             // replaced in finish_contact
            kind: PartyKind::Other, // replaced in finish_contact
            preferred_contact_method: row.get(7)?,
            address_line1: row.get(8)?,
            address_line2: row.get(9)?,
            city: row.get(10)?,
            state: row.get(11)?,
            postal_code: row.get(12)?,
            property_type: row.get(13)?,
            notes: row.get(14)?,
            favorite: row.get(15)?,
            archived_at: row.get(16)?,
            created_at: row.get(17)?,
            updated_at: row.get(18)?,
            version: row.get(19)?,
            channels: Vec::new(),
        },
        kind_text,
        role_text,
    ))
}

/// Parse stored enum text once the rusqlite row mapping is done.
fn finish_contact(
    (mut contact, kind_text, role_text): ContactRow,
) -> Result<Contact, ApplicationError> {
    contact.kind = PartyKind::from_database_value(&kind_text).ok_or_else(|| {
        ApplicationError::InvalidStoredData(format!(
            "contact {} has unsupported kind {kind_text}",
            contact.id
        ))
    })?;
    contact.role = match role_text {
        None => None,
        Some(role_text) => Some(ContactRole::from_database_value(&role_text).ok_or_else(|| {
            ApplicationError::InvalidStoredData(format!(
                "contact {} has unsupported role {role_text}",
                contact.id
            ))
        })?),
    };
    Ok(contact)
}

/// Read a contact and its channels inside the current transaction.
fn require_contact(
    transaction: &Transaction<'_>,
    contact_id: &str,
) -> Result<Contact, ApplicationError> {
    let row = transaction
        .query_row(
            &format!("SELECT {CONTACT_COLUMNS} FROM contacts WHERE id = ?1"),
            [contact_id],
            contact_from_row,
        )
        .optional()?
        .ok_or_else(|| ApplicationError::NotFound {
            resource: "contact",
            id: contact_id.into(),
        })?;
    let mut contact = finish_contact(row)?;
    contact.channels = load_channels(transaction, &contact.id)?;
    Ok(contact)
}

/// Insert validated channels in input order; sort_key follows that order.
fn insert_channels(
    transaction: &Transaction<'_>,
    contact_id: &str,
    channels: &[ValidChannel],
) -> Result<(), ApplicationError> {
    for (sort_key, channel) in channels.iter().enumerate() {
        transaction.execute(
            "INSERT INTO contact_channels (id, contact_id, kind, label, value, preferred, sort_key)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                new_id(),
                contact_id,
                channel.kind.as_database_value(),
                channel.label,
                channel.value,
                channel.preferred,
                sort_key as i64,
            ],
        )?;
    }
    Ok(())
}

fn load_channels(
    connection: &rusqlite::Connection,
    contact_id: &str,
) -> Result<Vec<ContactChannel>, ApplicationError> {
    let mut statement = connection.prepare(
        "SELECT id, contact_id, kind, label, value, preferred, sort_key
         FROM contact_channels WHERE contact_id = ?1 ORDER BY sort_key, id",
    )?;
    let rows = statement
        .query_map([contact_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, bool>(5)?,
                row.get::<_, i64>(6)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(
            |(id, contact_id, kind_text, label, value, preferred, sort_key)| {
                let kind = ChannelKind::from_database_value(&kind_text).ok_or_else(|| {
                    ApplicationError::InvalidStoredData(format!(
                        "channel {id} has unsupported kind {kind_text}"
                    ))
                })?;
                Ok(ContactChannel {
                    id,
                    contact_id,
                    kind,
                    label,
                    value,
                    preferred,
                    sort_key,
                })
            },
        )
        .collect()
}

/// Append a command_log row for undo/audit; part of the mutation transaction.
fn log_command(
    transaction: &Transaction<'_>,
    actor: Actor,
    entity_type: &str,
    entity_id: &str,
    summary: &str,
) -> Result<(), ApplicationError> {
    transaction.execute(
        "INSERT INTO command_log (id, command_id, actor, entity_type, entity_id, summary, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            new_id(),
            new_id(),
            actor.as_database_value(),
            entity_type,
            entity_id,
            summary,
            now_utc(),
        ],
    )?;
    Ok(())
}
