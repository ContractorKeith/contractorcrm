//! Application use-cases for companies and contacts. Every mutation runs in
//! one immediate transaction, checks the expected record version, bumps the
//! version, and writes a command_log row before committing.

use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};

use crate::attention::{
    self, AttentionFlag, AttentionInputs, ContactFacts, OpportunityFacts, TaskFacts, Thresholds,
};
use crate::domain::{
    Activity, ActivityDirection, ActivityKind, Actor, ChannelKind, Company, Contact,
    ContactChannel, ContactRole, HandoffRef, LostReason, Money, Opportunity, OpportunitySource,
    ParentType, PartyKind, Stage, StageHistoryEntry, StageKind, Task, TaskPriority, TaskStatus,
};
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

/// List row for the contact table — record plus read-time projections
/// (computed from activities and tasks, never stored).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactListItem {
    #[serde(flatten)]
    pub contact: Contact,
    /// Latest activity on the contact or any of its opportunities.
    pub last_contacted_at: Option<String>,
    /// Earliest due date among the contact's open tasks.
    pub next_open_task_due_at: Option<String>,
}

/// List contacts with their channels and read-time projections; archived rows
/// only when asked for.
pub fn list_contacts(
    storage: &Storage,
    include_archived: bool,
) -> Result<Vec<ContactListItem>, ApplicationError> {
    let connection = storage.connection();
    let mut statement = connection.prepare(&format!(
        "SELECT {CONTACT_COLUMNS},
                (SELECT MAX(a.occurred_at) FROM activities a
                 WHERE (a.parent_type = 'contact' AND a.parent_id = c.id)
                    OR (a.parent_type = 'opportunity' AND a.parent_id IN
                        (SELECT o.id FROM opportunities o WHERE o.contact_id = c.id))),
                (SELECT MIN(t.due_at) FROM tasks t
                 WHERE t.status = 'open' AND t.parent_type = 'contact'
                   AND t.parent_id = c.id AND t.due_at IS NOT NULL)
         FROM contacts c {} ORDER BY display_name, id",
        if include_archived {
            ""
        } else {
            "WHERE archived_at IS NULL"
        }
    ))?;
    let rows = statement
        .query_map([], |row| {
            let base = contact_from_row(row)?;
            let last_contacted_at: Option<String> = row.get(20)?;
            let next_open_task_due_at: Option<String> = row.get(21)?;
            Ok((base, last_contacted_at, next_open_task_due_at))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(row, last_contacted_at, next_open_task_due_at)| {
            let mut contact = finish_contact(row)?;
            contact.channels = load_channels(connection, &contact.id)?;
            Ok(ContactListItem {
                contact,
                last_contacted_at,
                next_open_task_due_at,
            })
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
// Pipeline requests (camelCase)
// ---------------------------------------------------------------------------

/// Rename or reorder one stage; kind and pipeline are fixed in v1.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStageRequest {
    #[serde(default)]
    pub actor: Actor,
    pub stage_id: String,
    pub expected_version: i64,
    pub name: String,
    pub sort_key: i64,
}

/// Editable opportunity fields; updates replace the full editable set (v1).
/// Stage changes go through `move_opportunity_stage`, never through updates.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityPatch {
    pub name: String,
    pub contact_id: Option<String>,
    pub company_id: Option<String>,
    #[serde(default)]
    pub value_minor: i64,
    /// ISO 4217 code, e.g. "USD"; validated as three letters.
    pub currency_code: String,
    pub probability_percent: Option<i64>,
    pub expected_close_date: Option<String>,
    /// Wire enum value, e.g. "referral"; optional.
    pub source: Option<String>,
    pub source_label: Option<String>,
    pub notes: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateOpportunityRequest {
    #[serde(default)]
    pub actor: Actor,
    /// Starting stage; defaults to the pipeline's first open stage.
    pub stage_id: Option<String>,
    #[serde(flatten)]
    pub opportunity: OpportunityPatch,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateOpportunityRequest {
    #[serde(default)]
    pub actor: Actor,
    pub opportunity_id: String,
    pub expected_version: i64,
    pub patch: OpportunityPatch,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveOpportunityStageRequest {
    #[serde(default)]
    pub actor: Actor,
    pub opportunity_id: String,
    pub to_stage_id: String,
    /// Required when the target stage kind is `lost`.
    pub lost_reason_id: Option<String>,
    pub expected_version: i64,
}

/// Table row for the opportunity list — record plus display names.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityListItem {
    #[serde(flatten)]
    pub opportunity: Opportunity,
    pub stage_name: String,
    pub contact_display_name: Option<String>,
    pub company_name: Option<String>,
    /// Latest activity logged on the opportunity (read-time, never stored).
    pub last_contacted_at: Option<String>,
    /// Earliest due date among the opportunity's open tasks.
    pub next_open_task_due_at: Option<String>,
}

/// Detail view — the record plus its full append-only stage history.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpportunityDetail {
    #[serde(flatten)]
    pub opportunity: Opportunity,
    pub stage_history: Vec<StageHistoryEntry>,
}

// ---------------------------------------------------------------------------
// Stage and lost-reason use-cases
// ---------------------------------------------------------------------------

/// List every stage in pipeline order.
pub fn list_stages(storage: &Storage) -> Result<Vec<Stage>, ApplicationError> {
    let mut statement = storage.connection().prepare(&format!(
        "SELECT {STAGE_COLUMNS} FROM stages ORDER BY pipeline_id, sort_key, id"
    ))?;
    let rows = statement
        .query_map([], stage_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter().map(finish_stage).collect()
}

/// Rename or reorder a stage; history keeps its ids, so it never changes.
pub fn update_stage(
    storage: &mut Storage,
    request: UpdateStageRequest,
) -> Result<Stage, ApplicationError> {
    let name = required_text("name", request.name, 100)?;
    if request.sort_key < 0 {
        return Err(ApplicationError::InvalidInput {
            field: "sortKey".into(),
            message: "must be zero or greater".into(),
        });
    }
    let transaction = immediate(storage)?;
    let mut stage = require_stage(&transaction, &request.stage_id)?;
    check_version("stage", &stage.id, request.expected_version, stage.version)?;

    stage.name = name;
    stage.sort_key = request.sort_key;
    stage.updated_at = now_utc();
    stage.version += 1;
    transaction.execute(
        "UPDATE stages SET name = ?2, sort_key = ?3, updated_at = ?4, version = ?5 WHERE id = ?1",
        params![
            stage.id,
            stage.name,
            stage.sort_key,
            stage.updated_at,
            stage.version,
        ],
    )?;
    log_command(
        &transaction,
        request.actor,
        "stage",
        &stage.id,
        &format!("updated stage \"{}\"", stage.name),
    )?;
    transaction.commit()?;
    Ok(stage)
}

/// List all lost reasons in sort order, active or not (UI filters).
pub fn list_lost_reasons(storage: &Storage) -> Result<Vec<LostReason>, ApplicationError> {
    let mut statement = storage
        .connection()
        .prepare("SELECT id, label, sort_key, active FROM lost_reasons ORDER BY sort_key, id")?;
    let reasons = statement
        .query_map([], |row| {
            Ok(LostReason {
                id: row.get(0)?,
                label: row.get(1)?,
                sort_key: row.get(2)?,
                active: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(reasons)
}

// ---------------------------------------------------------------------------
// Opportunity use-cases
// ---------------------------------------------------------------------------

/// Create an opportunity in its starting stage, record the first stage_history
/// row, and log the command.
pub fn create_opportunity(
    storage: &mut Storage,
    request: CreateOpportunityRequest,
) -> Result<Opportunity, ApplicationError> {
    let fields = validate_opportunity_patch(request.opportunity)?;
    let now = now_utc();
    let opportunity_id = new_id();

    let transaction = immediate(storage)?;
    require_linked_contact(&transaction, fields.contact_id.as_deref())?;
    require_linked_company(&transaction, fields.company_id.as_deref())?;
    let stage = match optional_text(request.stage_id) {
        Some(stage_id) => require_stage(&transaction, &stage_id)?,
        None => first_open_stage(&transaction)?,
    };
    transaction.execute(
        "INSERT INTO opportunities (
            id, name, contact_id, company_id, stage_id, value_minor, currency_code,
            probability_percent, expected_close_date, source, source_label,
            lost_reason_id, notes, archived_at, created_at, updated_at, version
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                   NULL, ?12, NULL, ?13, ?13, 1)",
        params![
            opportunity_id,
            fields.name,
            fields.contact_id,
            fields.company_id,
            stage.id,
            fields.value.value_minor,
            fields.value.currency_code,
            fields.probability_percent,
            fields.expected_close_date,
            fields.source.map(OpportunitySource::as_database_value),
            fields.source_label,
            fields.notes,
            now,
        ],
    )?;
    insert_stage_history(
        &transaction,
        &opportunity_id,
        None,
        &stage.id,
        request.actor,
        None,
    )?;
    log_command(
        &transaction,
        request.actor,
        "opportunity",
        &opportunity_id,
        &format!("created opportunity \"{}\"", fields.name),
    )?;
    let opportunity = require_opportunity(&transaction, &opportunity_id)?;
    transaction.commit()?;
    Ok(opportunity)
}

/// Replace an opportunity's editable fields; the stage and lost reason are
/// untouched — those move through `move_opportunity_stage`.
pub fn update_opportunity(
    storage: &mut Storage,
    request: UpdateOpportunityRequest,
) -> Result<Opportunity, ApplicationError> {
    let fields = validate_opportunity_patch(request.patch)?;
    let transaction = immediate(storage)?;
    let existing = require_opportunity(&transaction, &request.opportunity_id)?;
    check_version(
        "opportunity",
        &existing.id,
        request.expected_version,
        existing.version,
    )?;
    require_linked_contact(&transaction, fields.contact_id.as_deref())?;
    require_linked_company(&transaction, fields.company_id.as_deref())?;

    transaction.execute(
        "UPDATE opportunities SET
            name = ?2, contact_id = ?3, company_id = ?4, value_minor = ?5,
            currency_code = ?6, probability_percent = ?7, expected_close_date = ?8,
            source = ?9, source_label = ?10, notes = ?11, updated_at = ?12,
            version = ?13
         WHERE id = ?1",
        params![
            existing.id,
            fields.name,
            fields.contact_id,
            fields.company_id,
            fields.value.value_minor,
            fields.value.currency_code,
            fields.probability_percent,
            fields.expected_close_date,
            fields.source.map(OpportunitySource::as_database_value),
            fields.source_label,
            fields.notes,
            now_utc(),
            existing.version + 1,
        ],
    )?;
    log_command(
        &transaction,
        request.actor,
        "opportunity",
        &existing.id,
        &format!("updated opportunity \"{}\"", fields.name),
    )?;
    let opportunity = require_opportunity(&transaction, &existing.id)?;
    transaction.commit()?;
    Ok(opportunity)
}

/// Archive an opportunity; history stays, the record leaves default lists.
pub fn archive_opportunity(
    storage: &mut Storage,
    request: ArchiveRequest,
) -> Result<Opportunity, ApplicationError> {
    set_opportunity_archived(storage, request, true)
}

/// Bring an archived opportunity back.
pub fn unarchive_opportunity(
    storage: &mut Storage,
    request: ArchiveRequest,
) -> Result<Opportunity, ApplicationError> {
    set_opportunity_archived(storage, request, false)
}

fn set_opportunity_archived(
    storage: &mut Storage,
    request: ArchiveRequest,
    archived: bool,
) -> Result<Opportunity, ApplicationError> {
    let transaction = immediate(storage)?;
    let mut opportunity = require_opportunity(&transaction, &request.id)?;
    check_version(
        "opportunity",
        &opportunity.id,
        request.expected_version,
        opportunity.version,
    )?;

    opportunity.archived_at = archived.then(now_utc);
    opportunity.updated_at = now_utc();
    opportunity.version += 1;
    transaction.execute(
        "UPDATE opportunities SET archived_at = ?2, updated_at = ?3, version = ?4 WHERE id = ?1",
        params![
            opportunity.id,
            opportunity.archived_at,
            opportunity.updated_at,
            opportunity.version,
        ],
    )?;
    let verb = if archived { "archived" } else { "unarchived" };
    log_command(
        &transaction,
        request.actor,
        "opportunity",
        &opportunity.id,
        &format!("{verb} opportunity \"{}\"", opportunity.name),
    )?;
    transaction.commit()?;
    Ok(opportunity)
}

/// Move an opportunity to another stage, appending a stage_history row in the
/// same transaction. Lost moves require a reason; leaving lost clears it.
pub fn move_opportunity_stage(
    storage: &mut Storage,
    request: MoveOpportunityStageRequest,
) -> Result<Opportunity, ApplicationError> {
    let transaction = immediate(storage)?;
    let existing = require_opportunity(&transaction, &request.opportunity_id)?;
    check_version(
        "opportunity",
        &existing.id,
        request.expected_version,
        existing.version,
    )?;
    let to_stage = require_stage(&transaction, &request.to_stage_id)?;

    // Lost stage requires a reason; every other stage clears any stored one.
    let lost_reason_id = if to_stage.kind == StageKind::Lost {
        let reason_id = optional_text(request.lost_reason_id).ok_or_else(|| {
            ApplicationError::MissingLostReason {
                id: existing.id.clone(),
            }
        })?;
        require_lost_reason(&transaction, &reason_id)?;
        Some(reason_id)
    } else {
        None
    };

    transaction.execute(
        "UPDATE opportunities SET
            stage_id = ?2, lost_reason_id = ?3, updated_at = ?4, version = ?5
         WHERE id = ?1",
        params![
            existing.id,
            to_stage.id,
            lost_reason_id,
            now_utc(),
            existing.version + 1,
        ],
    )?;
    insert_stage_history(
        &transaction,
        &existing.id,
        Some(&existing.stage_id),
        &to_stage.id,
        request.actor,
        lost_reason_id.as_deref(),
    )?;
    log_command(
        &transaction,
        request.actor,
        "opportunity",
        &existing.id,
        &format!(
            "moved opportunity \"{}\" to stage \"{}\"",
            existing.name, to_stage.name
        ),
    )?;
    let opportunity = require_opportunity(&transaction, &existing.id)?;
    transaction.commit()?;
    Ok(opportunity)
}

/// List opportunities with stage and party display names for the table view;
/// archived rows only when asked for.
pub fn list_opportunities(
    storage: &Storage,
    include_archived: bool,
) -> Result<Vec<OpportunityListItem>, ApplicationError> {
    let mut statement = storage.connection().prepare(&format!(
        "SELECT o.id, o.name, o.contact_id, o.company_id, o.stage_id, o.value_minor,
                o.currency_code, o.probability_percent, o.expected_close_date, o.source,
                o.source_label, o.lost_reason_id, o.notes,
                o.quote_tool, o.quote_external_id, o.quote_label, o.quote_linked_at,
                o.job_tool, o.job_external_id, o.job_label, o.job_linked_at,
                o.archived_at, o.created_at, o.updated_at, o.version,
                s.name, c.display_name, co.name,
                (SELECT MAX(a.occurred_at) FROM activities a
                 WHERE a.parent_type = 'opportunity' AND a.parent_id = o.id),
                (SELECT MIN(t.due_at) FROM tasks t
                 WHERE t.status = 'open' AND t.parent_type = 'opportunity'
                   AND t.parent_id = o.id AND t.due_at IS NOT NULL)
         FROM opportunities o
         JOIN stages s ON s.id = o.stage_id
         LEFT JOIN contacts c ON c.id = o.contact_id
         LEFT JOIN companies co ON co.id = o.company_id
         {} ORDER BY o.name, o.id",
        if include_archived {
            ""
        } else {
            "WHERE o.archived_at IS NULL"
        }
    ))?;
    let rows = statement
        .query_map([], |row| {
            let base = opportunity_from_row(row)?;
            let stage_name: String = row.get(25)?;
            let contact_display_name: Option<String> = row.get(26)?;
            let company_name: Option<String> = row.get(27)?;
            let last_contacted_at: Option<String> = row.get(28)?;
            let next_open_task_due_at: Option<String> = row.get(29)?;
            Ok((
                base,
                stage_name,
                contact_display_name,
                company_name,
                last_contacted_at,
                next_open_task_due_at,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(
            |(
                base,
                stage_name,
                contact_display_name,
                company_name,
                last_contacted_at,
                next_open_task_due_at,
            )| {
                Ok(OpportunityListItem {
                    opportunity: finish_opportunity(base)?,
                    stage_name,
                    contact_display_name,
                    company_name,
                    last_contacted_at,
                    next_open_task_due_at,
                })
            },
        )
        .collect()
}

/// Fetch one opportunity with its full stage history, archived or not.
pub fn get_opportunity(
    storage: &Storage,
    opportunity_id: &str,
) -> Result<OpportunityDetail, ApplicationError> {
    let connection = storage.connection();
    let row = connection
        .query_row(
            &format!("SELECT {OPPORTUNITY_COLUMNS} FROM opportunities WHERE id = ?1"),
            [opportunity_id],
            opportunity_from_row,
        )
        .optional()?
        .ok_or_else(|| ApplicationError::NotFound {
            resource: "opportunity",
            id: opportunity_id.into(),
        })?;
    Ok(OpportunityDetail {
        opportunity: finish_opportunity(row)?,
        stage_history: load_stage_history(connection, opportunity_id)?,
    })
}

// ---------------------------------------------------------------------------
// Activity requests (camelCase)
// ---------------------------------------------------------------------------

/// Editable activity fields; updates replace the full editable set (v1).
/// The parent never changes after logging.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityPatch {
    /// Wire enum value, e.g. "call"; validated by the application layer.
    pub kind: String,
    /// Wire enum value; defaults to "none" when absent.
    pub direction: Option<String>,
    /// User-editable UTC ISO-8601 timestamp; defaults to now when absent.
    pub occurred_at: Option<String>,
    pub summary: String,
    /// Markdown, optional.
    pub body: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogActivityRequest {
    #[serde(default)]
    pub actor: Actor,
    /// Wire enum value: "contact", "company", or "opportunity".
    pub parent_type: String,
    pub parent_id: String,
    #[serde(flatten)]
    pub activity: ActivityPatch,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateActivityRequest {
    #[serde(default)]
    pub actor: Actor,
    pub activity_id: String,
    pub expected_version: i64,
    pub patch: ActivityPatch,
}

/// Hard delete — activities are user notes, so there is no archive state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteActivityRequest {
    #[serde(default)]
    pub actor: Actor,
    pub activity_id: String,
    pub expected_version: i64,
}

// ---------------------------------------------------------------------------
// Activity use-cases (unified timeline)
// ---------------------------------------------------------------------------

/// Log an activity on a contact, company, or opportunity; the parent must
/// exist (validated here — the table has no FK on the polymorphic parent_id).
pub fn log_activity(
    storage: &mut Storage,
    request: LogActivityRequest,
) -> Result<Activity, ApplicationError> {
    let parent_type = parse_parent_type("parentType", &request.parent_type)?;
    let parent_id = required_text("parentId", request.parent_id, 100)?;
    let fields = validate_activity_patch(request.activity)?;
    let now = now_utc();
    let activity = Activity {
        id: new_id(),
        parent_type,
        parent_id,
        kind: fields.kind,
        direction: fields.direction,
        occurred_at: fields.occurred_at,
        summary: fields.summary,
        body: fields.body,
        actor: request.actor,
        created_at: now.clone(),
        updated_at: now,
        version: 1,
    };

    let transaction = immediate(storage)?;
    require_activity_parent(&transaction, parent_type, &activity.parent_id)?;
    transaction.execute(
        "INSERT INTO activities (
            id, parent_type, parent_id, kind, direction, occurred_at,
            summary, body, actor, created_at, updated_at, version
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            activity.id,
            activity.parent_type.as_database_value(),
            activity.parent_id,
            activity.kind.as_database_value(),
            activity.direction.as_database_value(),
            activity.occurred_at,
            activity.summary,
            activity.body,
            activity.actor.as_database_value(),
            activity.created_at,
            activity.updated_at,
            activity.version,
        ],
    )?;
    log_command(
        &transaction,
        request.actor,
        "activity",
        &activity.id,
        &format!(
            "logged {} activity \"{}\"",
            activity.kind.as_database_value(),
            activity.summary
        ),
    )?;
    transaction.commit()?;
    Ok(activity)
}

/// Replace an activity's editable fields; the parent and original actor stay
/// fixed. Requires the expected version.
pub fn update_activity(
    storage: &mut Storage,
    request: UpdateActivityRequest,
) -> Result<Activity, ApplicationError> {
    let fields = validate_activity_patch(request.patch)?;
    let transaction = immediate(storage)?;
    let mut activity = require_activity(&transaction, &request.activity_id)?;
    check_version(
        "activity",
        &activity.id,
        request.expected_version,
        activity.version,
    )?;

    activity.kind = fields.kind;
    activity.direction = fields.direction;
    activity.occurred_at = fields.occurred_at;
    activity.summary = fields.summary;
    activity.body = fields.body;
    activity.updated_at = now_utc();
    activity.version += 1;
    transaction.execute(
        "UPDATE activities SET
            kind = ?2, direction = ?3, occurred_at = ?4, summary = ?5,
            body = ?6, updated_at = ?7, version = ?8
         WHERE id = ?1",
        params![
            activity.id,
            activity.kind.as_database_value(),
            activity.direction.as_database_value(),
            activity.occurred_at,
            activity.summary,
            activity.body,
            activity.updated_at,
            activity.version,
        ],
    )?;
    log_command(
        &transaction,
        request.actor,
        "activity",
        &activity.id,
        &format!("updated activity \"{}\"", activity.summary),
    )?;
    transaction.commit()?;
    Ok(activity)
}

/// Hard-delete an activity (they are user notes — no archive convention);
/// requires the expected version and logs the command.
pub fn delete_activity(
    storage: &mut Storage,
    request: DeleteActivityRequest,
) -> Result<(), ApplicationError> {
    let transaction = immediate(storage)?;
    let activity = require_activity(&transaction, &request.activity_id)?;
    check_version(
        "activity",
        &activity.id,
        request.expected_version,
        activity.version,
    )?;
    transaction.execute("DELETE FROM activities WHERE id = ?1", [&activity.id])?;
    log_command(
        &transaction,
        request.actor,
        "activity",
        &activity.id,
        &format!("deleted activity \"{}\"", activity.summary),
    )?;
    transaction.commit()?;
    Ok(())
}

/// Read a parent's timeline, newest first by user-editable occurred_at. With
/// `include_related`, a contact's timeline also carries activities of
/// opportunities linked via opportunities.contact_id (a company's via
/// company_id) — stored once on the opportunity, joined at read time.
pub fn get_timeline(
    storage: &Storage,
    parent_type: &str,
    parent_id: &str,
    include_related: bool,
) -> Result<Vec<Activity>, ApplicationError> {
    let parent_type = parse_parent_type("parentType", parent_type)?;
    let connection = storage.connection();
    require_activity_parent(connection, parent_type, parent_id)?;

    // Which opportunities column links related activities back to this parent.
    let related_link_column = match parent_type {
        ParentType::Contact => Some("contact_id"),
        ParentType::Company => Some("company_id"),
        ParentType::Opportunity => None,
    };
    let mut sql = format!(
        "SELECT {ACTIVITY_COLUMNS} FROM activities
         WHERE (parent_type = ?1 AND parent_id = ?2)"
    );
    if include_related {
        if let Some(column) = related_link_column {
            sql.push_str(&format!(
                " OR (parent_type = 'opportunity' AND parent_id IN
                     (SELECT id FROM opportunities WHERE {column} = ?2))"
            ));
        }
    }
    sql.push_str(" ORDER BY occurred_at DESC, id DESC");

    let mut statement = connection.prepare(&sql)?;
    let rows = statement
        .query_map(
            params![parent_type.as_database_value(), parent_id],
            activity_from_row,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter().map(finish_activity).collect()
}

// ---------------------------------------------------------------------------
// Task requests (camelCase)
// ---------------------------------------------------------------------------

/// Editable task fields; updates replace the full editable set (v1).
/// Status moves through complete/reopen/drop, never through updates.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskPatch {
    pub title: String,
    /// Markdown, optional.
    pub body: Option<String>,
    /// Wire enum value: "contact", "company", or "opportunity"; optional —
    /// personal tasks have none. Set together with parentId or not at all.
    pub parent_type: Option<String>,
    pub parent_id: Option<String>,
    /// UTC ISO-8601 timestamps, both optional.
    pub due_at: Option<String>,
    pub remind_at: Option<String>,
    /// Wire enum value; defaults to "normal" when absent.
    pub priority: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTaskRequest {
    #[serde(default)]
    pub actor: Actor,
    #[serde(flatten)]
    pub task: TaskPatch,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTaskRequest {
    #[serde(default)]
    pub actor: Actor,
    pub task_id: String,
    pub expected_version: i64,
    pub patch: TaskPatch,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompleteTaskRequest {
    #[serde(default)]
    pub actor: Actor,
    pub task_id: String,
    pub expected_version: i64,
    /// Also log a "Completed task: …" note on the task's parent in the same
    /// transaction; invalid for a task with no parent.
    #[serde(default)]
    pub log_activity: bool,
}

/// Shared shape for reopen, drop, and hard delete of a task.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskActionRequest {
    #[serde(default)]
    pub actor: Actor,
    pub task_id: String,
    pub expected_version: i64,
}

/// Filter shape for `list_tasks`: one optional status ("open", "done", or
/// "dropped"; absent means every status — this replaces a separate
/// include_done flag), an overdue-only switch (implies open + past due_at),
/// and an optional parent (both fields together or neither).
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTasksRequest {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub overdue_only: bool,
    #[serde(default)]
    pub parent_type: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Task use-cases
// ---------------------------------------------------------------------------

/// Create a task; a linked parent is optional but must exist when given
/// (validated here — the table has no FK on the polymorphic parent_id).
pub fn create_task(
    storage: &mut Storage,
    request: CreateTaskRequest,
) -> Result<Task, ApplicationError> {
    let fields = validate_task_patch(request.task)?;
    let now = now_utc();
    let task_id = new_id();

    let transaction = immediate(storage)?;
    if let Some((parent_type, parent_id)) = &fields.parent {
        require_activity_parent(&transaction, *parent_type, parent_id)?;
    }
    transaction.execute(
        "INSERT INTO tasks (
            id, title, body, parent_type, parent_id, due_at, remind_at,
            priority, status, completed_at, created_at, updated_at, version
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'open', NULL, ?9, ?9, 1)",
        params![
            task_id,
            fields.title,
            fields.body,
            fields
                .parent
                .as_ref()
                .map(|(parent_type, _)| parent_type.as_database_value()),
            fields.parent.as_ref().map(|(_, parent_id)| parent_id),
            fields.due_at,
            fields.remind_at,
            fields.priority.as_database_value(),
            now,
        ],
    )?;
    log_command(
        &transaction,
        request.actor,
        "task",
        &task_id,
        &format!("created task \"{}\"", fields.title),
    )?;
    let task = require_task(&transaction, &task_id)?;
    transaction.commit()?;
    Ok(task)
}

/// Replace a task's editable fields (title/body/parent/due/remind/priority);
/// status and completed_at only move through complete/reopen/drop.
pub fn update_task(
    storage: &mut Storage,
    request: UpdateTaskRequest,
) -> Result<Task, ApplicationError> {
    let fields = validate_task_patch(request.patch)?;
    let transaction = immediate(storage)?;
    let existing = require_task(&transaction, &request.task_id)?;
    check_version(
        "task",
        &existing.id,
        request.expected_version,
        existing.version,
    )?;
    if let Some((parent_type, parent_id)) = &fields.parent {
        require_activity_parent(&transaction, *parent_type, parent_id)?;
    }

    transaction.execute(
        "UPDATE tasks SET
            title = ?2, body = ?3, parent_type = ?4, parent_id = ?5,
            due_at = ?6, remind_at = ?7, priority = ?8, updated_at = ?9,
            version = ?10
         WHERE id = ?1",
        params![
            existing.id,
            fields.title,
            fields.body,
            fields
                .parent
                .as_ref()
                .map(|(parent_type, _)| parent_type.as_database_value()),
            fields.parent.as_ref().map(|(_, parent_id)| parent_id),
            fields.due_at,
            fields.remind_at,
            fields.priority.as_database_value(),
            now_utc(),
            existing.version + 1,
        ],
    )?;
    log_command(
        &transaction,
        request.actor,
        "task",
        &existing.id,
        &format!("updated task \"{}\"", fields.title),
    )?;
    let task = require_task(&transaction, &existing.id)?;
    transaction.commit()?;
    Ok(task)
}

/// Mark an open task done, stamping completed_at. With `log_activity`, a
/// "Completed task: …" note lands on the task's parent in the same
/// transaction; a parentless task cannot log one.
pub fn complete_task(
    storage: &mut Storage,
    request: CompleteTaskRequest,
) -> Result<Task, ApplicationError> {
    let transaction = immediate(storage)?;
    let existing = require_task(&transaction, &request.task_id)?;
    check_version(
        "task",
        &existing.id,
        request.expected_version,
        existing.version,
    )?;
    if existing.status != TaskStatus::Open {
        return Err(ApplicationError::ValidationFailed {
            code: "task_not_open",
            field: "taskId".into(),
            message: format!(
                "cannot complete task \"{}\": it is {}, not open",
                existing.title,
                existing.status.as_database_value()
            ),
        });
    }
    if request.log_activity && existing.parent_type.is_none() {
        return Err(ApplicationError::InvalidInput {
            field: "logActivity".into(),
            message: "cannot log an activity for a task with no parent".into(),
        });
    }

    let now = now_utc();
    transaction.execute(
        "UPDATE tasks SET status = 'done', completed_at = ?2, updated_at = ?2, version = ?3
         WHERE id = ?1",
        params![existing.id, now, existing.version + 1],
    )?;
    if request.log_activity {
        // Parent presence was checked above; copy it onto the note.
        let (parent_type, parent_id) = (
            existing.parent_type.expect("checked above"),
            existing.parent_id.clone().expect("checked above"),
        );
        let activity_id = new_id();
        let summary = format!("Completed task: {}", existing.title);
        transaction.execute(
            "INSERT INTO activities (
                id, parent_type, parent_id, kind, direction, occurred_at,
                summary, body, actor, created_at, updated_at, version
             ) VALUES (?1, ?2, ?3, 'note', 'none', ?4, ?5, NULL, ?6, ?4, ?4, 1)",
            params![
                activity_id,
                parent_type.as_database_value(),
                parent_id,
                now,
                summary,
                request.actor.as_database_value(),
            ],
        )?;
        log_command(
            &transaction,
            request.actor,
            "activity",
            &activity_id,
            &format!("logged note activity \"{summary}\""),
        )?;
    }
    log_command(
        &transaction,
        request.actor,
        "task",
        &existing.id,
        &format!("completed task \"{}\"", existing.title),
    )?;
    let task = require_task(&transaction, &existing.id)?;
    transaction.commit()?;
    Ok(task)
}

/// Bring a done or dropped task back to open, clearing completed_at.
pub fn reopen_task(
    storage: &mut Storage,
    request: TaskActionRequest,
) -> Result<Task, ApplicationError> {
    let transaction = immediate(storage)?;
    let existing = require_task(&transaction, &request.task_id)?;
    check_version(
        "task",
        &existing.id,
        request.expected_version,
        existing.version,
    )?;
    if existing.status == TaskStatus::Open {
        return Err(ApplicationError::ValidationFailed {
            code: "task_already_open",
            field: "taskId".into(),
            message: format!("task \"{}\" is already open", existing.title),
        });
    }

    transaction.execute(
        "UPDATE tasks SET status = 'open', completed_at = NULL, updated_at = ?2, version = ?3
         WHERE id = ?1",
        params![existing.id, now_utc(), existing.version + 1],
    )?;
    log_command(
        &transaction,
        request.actor,
        "task",
        &existing.id,
        &format!("reopened task \"{}\"", existing.title),
    )?;
    let task = require_task(&transaction, &existing.id)?;
    transaction.commit()?;
    Ok(task)
}

/// Drop an open task — abandoned, not done, so completed_at stays null.
pub fn drop_task(
    storage: &mut Storage,
    request: TaskActionRequest,
) -> Result<Task, ApplicationError> {
    let transaction = immediate(storage)?;
    let existing = require_task(&transaction, &request.task_id)?;
    check_version(
        "task",
        &existing.id,
        request.expected_version,
        existing.version,
    )?;
    if existing.status != TaskStatus::Open {
        return Err(ApplicationError::ValidationFailed {
            code: "task_not_open",
            field: "taskId".into(),
            message: format!(
                "cannot drop task \"{}\": it is {}, not open",
                existing.title,
                existing.status.as_database_value()
            ),
        });
    }

    transaction.execute(
        "UPDATE tasks SET status = 'dropped', updated_at = ?2, version = ?3 WHERE id = ?1",
        params![existing.id, now_utc(), existing.version + 1],
    )?;
    log_command(
        &transaction,
        request.actor,
        "task",
        &existing.id,
        &format!("dropped task \"{}\"", existing.title),
    )?;
    let task = require_task(&transaction, &existing.id)?;
    transaction.commit()?;
    Ok(task)
}

/// Hard-delete a task (they are user to-dos — no archive convention);
/// requires the expected version and logs the command.
pub fn delete_task(
    storage: &mut Storage,
    request: TaskActionRequest,
) -> Result<(), ApplicationError> {
    let transaction = immediate(storage)?;
    let existing = require_task(&transaction, &request.task_id)?;
    check_version(
        "task",
        &existing.id,
        request.expected_version,
        existing.version,
    )?;
    transaction.execute("DELETE FROM tasks WHERE id = ?1", [&existing.id])?;
    log_command(
        &transaction,
        request.actor,
        "task",
        &existing.id,
        &format!("deleted task \"{}\"", existing.title),
    )?;
    transaction.commit()?;
    Ok(())
}

/// List tasks ordered by due date (nulls last), then priority (high first),
/// then id. Overdue means status open with due_at before now — UTC ISO-8601
/// strings compare correctly as text.
pub fn list_tasks(
    storage: &Storage,
    request: ListTasksRequest,
) -> Result<Vec<Task>, ApplicationError> {
    let status = match optional_text(request.status) {
        None => None,
        Some(value) => Some(TaskStatus::from_database_value(&value).ok_or_else(|| {
            ApplicationError::InvalidInput {
                field: "status".into(),
                message: format!("unknown status \"{value}\"; expected one of open, done, dropped"),
            }
        })?),
    };
    let parent = parse_optional_parent(request.parent_type, request.parent_id)?;

    let mut clauses: Vec<String> = Vec::new();
    let mut binds: Vec<String> = Vec::new();
    if let Some(status) = status {
        clauses.push("status = ?".into());
        binds.push(status.as_database_value().into());
    }
    if request.overdue_only {
        clauses.push("status = 'open' AND due_at IS NOT NULL AND due_at < ?".into());
        binds.push(now_utc());
    }
    if let Some((parent_type, parent_id)) = parent {
        clauses.push("parent_type = ? AND parent_id = ?".into());
        binds.push(parent_type.as_database_value().into());
        binds.push(parent_id);
    }
    let where_sql = if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    };
    let mut statement = storage.connection().prepare(&format!(
        "SELECT {TASK_COLUMNS} FROM tasks {where_sql}
         ORDER BY (due_at IS NULL), due_at,
                  CASE priority WHEN 'high' THEN 0 WHEN 'normal' THEN 1 ELSE 2 END, id"
    ))?;
    let rows = statement
        .query_map(rusqlite::params_from_iter(binds.iter()), task_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter().map(finish_task).collect()
}

// ---------------------------------------------------------------------------
// Hand-off use-cases (quote/job references + envelope export)
// ---------------------------------------------------------------------------

/// Caller-supplied hand-off reference; `linked_at` is stamped on link.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HandoffRefInput {
    /// External tool name, e.g. "contractorproject".
    pub tool: String,
    /// The record's id inside that tool.
    pub external_id: String,
    pub label: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkQuoteRequest {
    #[serde(default)]
    pub actor: Actor,
    pub opportunity_id: String,
    pub expected_version: i64,
    pub quote_ref: HandoffRefInput,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkJobRequest {
    #[serde(default)]
    pub actor: Actor,
    pub opportunity_id: String,
    pub expected_version: i64,
    pub job_ref: HandoffRefInput,
}

/// Shared shape for clearing either hand-off reference.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnlinkHandoffRequest {
    #[serde(default)]
    pub actor: Actor,
    pub opportunity_id: String,
    pub expected_version: i64,
}

/// Which of the two hand-off references a link/unlink touches.
#[derive(Clone, Copy)]
enum HandoffKind {
    Quote,
    Job,
}

impl HandoffKind {
    fn noun(self) -> &'static str {
        match self {
            Self::Quote => "quote",
            Self::Job => "job",
        }
    }

    /// UPDATE statement writing this ref's four columns plus the bump.
    fn update_sql(self) -> &'static str {
        match self {
            Self::Quote => {
                "UPDATE opportunities SET
                    quote_tool = ?2, quote_external_id = ?3, quote_label = ?4,
                    quote_linked_at = ?5, updated_at = ?6, version = ?7
                 WHERE id = ?1"
            }
            Self::Job => {
                "UPDATE opportunities SET
                    job_tool = ?2, job_external_id = ?3, job_label = ?4,
                    job_linked_at = ?5, updated_at = ?6, version = ?7
                 WHERE id = ?1"
            }
        }
    }
}

/// Record a quote reference on an opportunity; version-checked and logged.
pub fn link_quote(
    storage: &mut Storage,
    request: LinkQuoteRequest,
) -> Result<Opportunity, ApplicationError> {
    set_handoff_ref(
        storage,
        HandoffKind::Quote,
        request.actor,
        &request.opportunity_id,
        request.expected_version,
        Some(request.quote_ref),
    )
}

/// Clear an opportunity's quote reference; version-checked and logged.
pub fn unlink_quote(
    storage: &mut Storage,
    request: UnlinkHandoffRequest,
) -> Result<Opportunity, ApplicationError> {
    set_handoff_ref(
        storage,
        HandoffKind::Quote,
        request.actor,
        &request.opportunity_id,
        request.expected_version,
        None,
    )
}

/// Record a ContractorProject job reference; only won opportunities qualify.
pub fn link_job(
    storage: &mut Storage,
    request: LinkJobRequest,
) -> Result<Opportunity, ApplicationError> {
    set_handoff_ref(
        storage,
        HandoffKind::Job,
        request.actor,
        &request.opportunity_id,
        request.expected_version,
        Some(request.job_ref),
    )
}

/// Clear an opportunity's job reference; version-checked and logged.
pub fn unlink_job(
    storage: &mut Storage,
    request: UnlinkHandoffRequest,
) -> Result<Opportunity, ApplicationError> {
    set_handoff_ref(
        storage,
        HandoffKind::Job,
        request.actor,
        &request.opportunity_id,
        request.expected_version,
        None,
    )
}

/// Shared link/unlink core: validate, enforce the won-stage rule for job
/// links, write the four ref columns, bump the version, and log the command.
fn set_handoff_ref(
    storage: &mut Storage,
    kind: HandoffKind,
    actor: Actor,
    opportunity_id: &str,
    expected_version: i64,
    reference: Option<HandoffRefInput>,
) -> Result<Opportunity, ApplicationError> {
    let reference = match reference {
        None => None,
        Some(input) => Some(validate_handoff_ref(input)?),
    };
    let transaction = immediate(storage)?;
    let existing = require_opportunity(&transaction, opportunity_id)?;
    check_version(
        "opportunity",
        &existing.id,
        expected_version,
        existing.version,
    )?;

    // Business rule: a job hand-off only makes sense once the work is won.
    if matches!(kind, HandoffKind::Job) && reference.is_some() {
        let stage = require_stage(&transaction, &existing.stage_id)?;
        if stage.kind != StageKind::Won {
            return Err(ApplicationError::ValidationFailed {
                code: "opportunity_not_won",
                field: "opportunityId".into(),
                message: format!(
                    "cannot link a job to opportunity \"{}\": it is in stage \"{}\", \
                     and job hand-offs require the won stage",
                    existing.name, stage.name
                ),
            });
        }
    }

    let (tool, external_id, label, linked_at) = match &reference {
        Some(reference) => (
            Some(reference.tool.as_str()),
            Some(reference.external_id.as_str()),
            reference.label.as_deref(),
            Some(now_utc()),
        ),
        None => (None, None, None, None),
    };
    transaction.execute(
        kind.update_sql(),
        params![
            existing.id,
            tool,
            external_id,
            label,
            linked_at,
            now_utc(),
            existing.version + 1,
        ],
    )?;
    let summary = match &reference {
        Some(reference) => format!(
            "linked {} {} to opportunity \"{}\"",
            kind.noun(),
            reference.label.as_deref().unwrap_or(&reference.external_id),
            existing.name
        ),
        None => format!(
            "unlinked {} from opportunity \"{}\"",
            kind.noun(),
            existing.name
        ),
    };
    log_command(&transaction, actor, "opportunity", &existing.id, &summary)?;
    let opportunity = require_opportunity(&transaction, &existing.id)?;
    transaction.commit()?;
    Ok(opportunity)
}

/// Hand-off ref rules: non-empty tool and external id, optional label.
fn validate_handoff_ref(input: HandoffRefInput) -> Result<HandoffRefInput, ApplicationError> {
    Ok(HandoffRefInput {
        tool: required_text("tool", input.tool, 100)?,
        external_id: required_text("externalId", input.external_id, 200)?,
        label: optional_text(input.label),
    })
}

// ---------------------------------------------------------------------------
// Hand-off envelope export
// ---------------------------------------------------------------------------

/// Envelope schema version — additive changes only within a major version
/// (docs/HANDOFF.md); breaking changes bump this number.
pub const HANDOFF_SCHEMA_VERSION: i64 = 1;

/// Product stamp inside every exported envelope.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductInfo {
    pub name: String,
    pub version: String,
}

/// Opportunity wire shape plus the resolved stage name for the envelope.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvelopeOpportunity {
    #[serde(flatten)]
    pub opportunity: Opportunity,
    pub stage_name: String,
}

/// Versioned opportunity hand-off envelope written by
/// `export_handoff_envelope` (schema in docs/HANDOFF.md).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HandoffEnvelope {
    pub schema_version: i64,
    pub kind: String,
    pub exported_at: String,
    pub product: ProductInfo,
    pub opportunity: EnvelopeOpportunity,
    pub contact: Option<Contact>,
    pub company: Option<Company>,
}

/// Where an envelope landed, for the UI and command log.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvelopeExportReport {
    pub destination_path: String,
    pub schema_version: i64,
}

/// Write a pretty-printed JSON hand-off envelope for one opportunity —
/// opportunity with money and refs, stage name, and linked contact/company.
/// Refuses an existing destination unless `overwrite` is set (mirrors
/// `backup_to`). Export is user-initiated, so the actor is always `user`.
pub fn export_handoff_envelope(
    storage: &mut Storage,
    opportunity_id: &str,
    destination_path: &str,
    overwrite: bool,
) -> Result<EnvelopeExportReport, ApplicationError> {
    let destination = required_text("destinationPath", destination_path.into(), 4096)?;
    let detail = get_opportunity(storage, opportunity_id)?;
    let opportunity = detail.opportunity;

    // Resolve linked records and the stage name outside any transaction —
    // reads only, matching the other query paths.
    let stage_name: String = storage.connection().query_row(
        "SELECT name FROM stages WHERE id = ?1",
        [&opportunity.stage_id],
        |row| row.get(0),
    )?;
    let contact = match &opportunity.contact_id {
        Some(contact_id) => Some(get_contact(storage, contact_id)?),
        None => None,
    };
    let company = match &opportunity.company_id {
        Some(company_id) => Some(get_company(storage, company_id)?),
        None => None,
    };

    let envelope = HandoffEnvelope {
        schema_version: HANDOFF_SCHEMA_VERSION,
        kind: "opportunity_handoff".into(),
        exported_at: now_utc(),
        product: ProductInfo {
            name: "ContractorCRM".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
        opportunity: EnvelopeOpportunity {
            opportunity,
            stage_name,
        },
        contact,
        company,
    };

    let destination_file = std::path::Path::new(&destination);
    if destination_file.exists() && !overwrite {
        return Err(ApplicationError::ValidationFailed {
            code: "destination_exists",
            field: "destinationPath".into(),
            message: format!("{destination} already exists; enable overwrite to replace it"),
        });
    }
    if let Some(parent) = destination_file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(&envelope)
        .map_err(|error| ApplicationError::InvalidStoredData(error.to_string()))?;
    std::fs::write(destination_file, json)?;

    let transaction = immediate(storage)?;
    log_command(
        &transaction,
        Actor::User,
        "opportunity",
        &envelope.opportunity.opportunity.id,
        &format!(
            "exported hand-off envelope for opportunity \"{}\" to \"{destination}\"",
            envelope.opportunity.opportunity.name
        ),
    )?;
    transaction.commit()?;
    Ok(EnvelopeExportReport {
        destination_path: destination,
        schema_version: HANDOFF_SCHEMA_VERSION,
    })
}

// ---------------------------------------------------------------------------
// Database maintenance use-cases (backup / restore / info)
// ---------------------------------------------------------------------------

/// app_settings key holding the last successful backup timestamp.
const LAST_BACKUP_AT_KEY: &str = "last_backup_at";

/// Storage-state report for the UI: where the database lives, how big it is,
/// and when it was last backed up.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseInfo {
    pub database_path: String,
    pub file_size_bytes: u64,
    pub last_backup_at: Option<String>,
}

/// Result of a successful restore, including where the safety copy went.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreReport {
    pub restored_from: String,
    pub safety_copy_path: String,
}

/// Snapshot the database to a caller-chosen path, record the timestamp in
/// app_settings, and log the command. Maintenance is user-initiated, so the
/// actor is always `user`.
pub fn backup_database(
    storage: &mut Storage,
    destination_path: &str,
    overwrite: bool,
) -> Result<DatabaseInfo, ApplicationError> {
    let destination = required_text("destinationPath", destination_path.into(), 4096)?;
    storage.backup_to(&destination, overwrite)?;

    let transaction = immediate(storage)?;
    transaction.execute(
        "INSERT INTO app_settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![LAST_BACKUP_AT_KEY, now_utc()],
    )?;
    log_command(
        &transaction,
        Actor::User,
        "database",
        "local",
        &format!("backed up database to \"{destination}\""),
    )?;
    transaction.commit()?;
    get_database_info(storage)
}

/// Restore the database from a verified backup file; the storage layer keeps a
/// pre-restore safety copy and migrates older backups forward. The command is
/// logged into the restored database so the event survives the swap.
pub fn restore_database(
    storage: &mut Storage,
    backup_path: &str,
) -> Result<RestoreReport, ApplicationError> {
    let backup_path = required_text("backupPath", backup_path.into(), 4096)?;
    let safety_copy = storage.restore_from(&backup_path)?;

    let transaction = immediate(storage)?;
    log_command(
        &transaction,
        Actor::User,
        "database",
        "local",
        &format!("restored database from \"{backup_path}\""),
    )?;
    transaction.commit()?;
    Ok(RestoreReport {
        restored_from: backup_path,
        safety_copy_path: safety_copy.to_string_lossy().into_owned(),
    })
}

/// Read-only storage-state report for the UI's storage display.
pub fn get_database_info(storage: &Storage) -> Result<DatabaseInfo, ApplicationError> {
    let file_size_bytes = std::fs::metadata(storage.database_path())?.len();
    let last_backup_at = storage
        .connection()
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            [LAST_BACKUP_AT_KEY],
            |row| row.get(0),
        )
        .optional()?;
    Ok(DatabaseInfo {
        database_path: storage.database_path().to_string_lossy().into_owned(),
        file_size_bytes,
        last_backup_at,
    })
}

// ---------------------------------------------------------------------------
// Needs-attention use-cases (thresholds + flag query)
// ---------------------------------------------------------------------------

/// app_settings keys for the attention thresholds; absent keys mean defaults.
const ATTENTION_STALE_LEAD_DAYS_KEY: &str = "attention.stale_lead_days";
const ATTENTION_PROPOSAL_DAYS_KEY: &str = "attention.proposal_no_response_days";
const ATTENTION_PROPOSAL_STAGE_KEY: &str = "attention.proposal_stage_name";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetAttentionThresholdsRequest {
    #[serde(default)]
    pub actor: Actor,
    pub stale_lead_days: i64,
    pub proposal_no_response_days: i64,
    /// Name of the stage the proposal rule watches; absent keeps the default.
    pub proposal_stage_name: Option<String>,
}

/// Read the attention thresholds from app_settings, falling back to the
/// defaults in `attention::Thresholds` for any absent key.
pub fn get_attention_thresholds(storage: &Storage) -> Result<Thresholds, ApplicationError> {
    let defaults = Thresholds::default();
    let connection = storage.connection();
    Ok(Thresholds {
        stale_lead_days: read_setting_days(
            connection,
            ATTENTION_STALE_LEAD_DAYS_KEY,
            defaults.stale_lead_days,
        )?,
        proposal_no_response_days: read_setting_days(
            connection,
            ATTENTION_PROPOSAL_DAYS_KEY,
            defaults.proposal_no_response_days,
        )?,
        proposal_stage_name: read_setting(connection, ATTENTION_PROPOSAL_STAGE_KEY)?
            .unwrap_or(defaults.proposal_stage_name),
    })
}

/// Persist the attention thresholds as individual app_settings keys; day
/// counts must be positive integers.
pub fn set_attention_thresholds(
    storage: &mut Storage,
    request: SetAttentionThresholdsRequest,
) -> Result<Thresholds, ApplicationError> {
    for (field, value) in [
        ("staleLeadDays", request.stale_lead_days),
        ("proposalNoResponseDays", request.proposal_no_response_days),
    ] {
        if value < 1 {
            return Err(ApplicationError::InvalidInput {
                field: field.into(),
                message: "must be a positive number of days".into(),
            });
        }
    }
    let proposal_stage_name = match request.proposal_stage_name {
        Some(name) => required_text("proposalStageName", name, 100)?,
        None => Thresholds::default().proposal_stage_name,
    };

    let transaction = immediate(storage)?;
    for (key, value) in [
        (
            ATTENTION_STALE_LEAD_DAYS_KEY,
            request.stale_lead_days.to_string(),
        ),
        (
            ATTENTION_PROPOSAL_DAYS_KEY,
            request.proposal_no_response_days.to_string(),
        ),
        (ATTENTION_PROPOSAL_STAGE_KEY, proposal_stage_name.clone()),
    ] {
        transaction.execute(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
    }
    log_command(
        &transaction,
        request.actor,
        "settings",
        "attention",
        "updated needs-attention thresholds",
    )?;
    transaction.commit()?;
    Ok(Thresholds {
        stale_lead_days: request.stale_lead_days,
        proposal_no_response_days: request.proposal_no_response_days,
        proposal_stage_name,
    })
}

/// Compute the needs-attention flags: gather the facts (last activity per
/// contact including related opportunities, stage entry times, open tasks) and
/// hand them to the pure rules in `attention`. `reference_time` defaults to
/// now; results are never stored.
pub fn get_attention_flags(
    storage: &Storage,
    reference_time: Option<String>,
) -> Result<Vec<AttentionFlag>, ApplicationError> {
    let reference_time = match optional_text(reference_time) {
        Some(value) => DateTime::parse_from_rfc3339(&value)
            .map(|parsed| parsed.with_timezone(&Utc))
            .map_err(|_| ApplicationError::InvalidInput {
                field: "referenceTime".into(),
                message: "must be a UTC ISO-8601 timestamp".into(),
            })?,
        None => Utc::now(),
    };
    let connection = storage.connection();
    let inputs = AttentionInputs {
        reference_time,
        thresholds: get_attention_thresholds(storage)?,
        contacts: load_contact_facts(connection)?,
        opportunities: load_opportunity_facts(connection)?,
        tasks: load_task_facts(connection)?,
    };
    Ok(attention::evaluate(&inputs))
}

/// Read one app_settings value, if present.
fn read_setting(
    connection: &rusqlite::Connection,
    key: &str,
) -> Result<Option<String>, ApplicationError> {
    Ok(connection
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            [key],
            |row| row.get(0),
        )
        .optional()?)
}

/// Read a stored day count; absent means the default, garbage is an error.
fn read_setting_days(
    connection: &rusqlite::Connection,
    key: &str,
    default: i64,
) -> Result<i64, ApplicationError> {
    match read_setting(connection, key)? {
        None => Ok(default),
        Some(value) => value
            .trim()
            .parse::<i64>()
            .ok()
            .filter(|days| *days > 0)
            .ok_or_else(|| {
                ApplicationError::InvalidStoredData(format!(
                    "app_settings {key} holds \"{value}\", not a positive day count"
                ))
            }),
    }
}

/// Parse a stored UTC ISO-8601 timestamp into a chrono value for the rules.
fn parse_stored_timestamp(context: &str, value: &str) -> Result<DateTime<Utc>, ApplicationError> {
    DateTime::parse_from_rfc3339(value)
        .map(|parsed| parsed.with_timezone(&Utc))
        .map_err(|_| {
            ApplicationError::InvalidStoredData(format!(
                "{context} holds \"{value}\", not a UTC ISO-8601 timestamp"
            ))
        })
}

/// Facts for the stale-lead rule: active contacts with the latest activity on
/// the contact itself or any opportunity linked to it.
fn load_contact_facts(
    connection: &rusqlite::Connection,
) -> Result<Vec<ContactFacts>, ApplicationError> {
    let mut statement = connection.prepare(
        "SELECT c.id, c.display_name, c.kind, c.created_at,
                (SELECT MAX(a.occurred_at) FROM activities a
                 WHERE (a.parent_type = 'contact' AND a.parent_id = c.id)
                    OR (a.parent_type = 'opportunity' AND a.parent_id IN
                        (SELECT o.id FROM opportunities o WHERE o.contact_id = c.id)))
         FROM contacts c WHERE c.archived_at IS NULL",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(id, display_name, kind, created_at, last_activity_at)| {
            Ok(ContactFacts {
                created_at: parse_stored_timestamp("contacts.created_at", &created_at)?,
                last_activity_at: last_activity_at
                    .map(|value| parse_stored_timestamp("activities.occurred_at", &value))
                    .transpose()?,
                id,
                display_name,
                kind,
            })
        })
        .collect()
}

/// Facts for the proposal rule: active opportunities with their current stage,
/// when they entered it (latest stage_history row, falling back to created_at),
/// and the latest inbound activity.
fn load_opportunity_facts(
    connection: &rusqlite::Connection,
) -> Result<Vec<OpportunityFacts>, ApplicationError> {
    let mut statement = connection.prepare(
        "SELECT o.id, o.name, s.kind, s.name,
                COALESCE((SELECT MAX(h.created_at) FROM stage_history h
                          WHERE h.opportunity_id = o.id), o.created_at),
                (SELECT MAX(a.occurred_at) FROM activities a
                 WHERE a.parent_type = 'opportunity' AND a.parent_id = o.id
                   AND a.direction = 'inbound')
         FROM opportunities o
         JOIN stages s ON s.id = o.stage_id
         WHERE o.archived_at IS NULL",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(
            |(id, name, stage_kind, stage_name, entered_at, last_inbound_at)| {
                Ok(OpportunityFacts {
                    stage_entered_at: parse_stored_timestamp(
                        "stage_history.created_at",
                        &entered_at,
                    )?,
                    last_inbound_activity_at: last_inbound_at
                        .map(|value| parse_stored_timestamp("activities.occurred_at", &value))
                        .transpose()?,
                    id,
                    name,
                    stage_kind,
                    stage_name,
                })
            },
        )
        .collect()
}

/// Facts for the overdue rule: open tasks with a due date.
fn load_task_facts(connection: &rusqlite::Connection) -> Result<Vec<TaskFacts>, ApplicationError> {
    let mut statement = connection.prepare(
        "SELECT id, title, status, due_at FROM tasks
         WHERE status = 'open' AND due_at IS NOT NULL",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(id, title, status, due_at)| {
            Ok(TaskFacts {
                due_at: Some(parse_stored_timestamp("tasks.due_at", &due_at)?),
                id,
                title,
                status,
            })
        })
        .collect()
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

/// Opportunity patch after validation, with parsed enums and typed money.
struct ValidOpportunityFields {
    name: String,
    contact_id: Option<String>,
    company_id: Option<String>,
    value: Money,
    probability_percent: Option<i64>,
    expected_close_date: Option<String>,
    source: Option<OpportunitySource>,
    source_label: Option<String>,
    notes: Option<String>,
}

fn validate_opportunity_patch(
    patch: OpportunityPatch,
) -> Result<ValidOpportunityFields, ApplicationError> {
    let contact_id = optional_text(patch.contact_id);
    let company_id = optional_text(patch.company_id);
    if contact_id.is_none() && company_id.is_none() {
        return Err(ApplicationError::ValidationFailed {
            code: "opportunity_needs_contact_or_company",
            field: "contactId".into(),
            message: "an opportunity needs a contact or a company (or both)".into(),
        });
    }
    if patch.value_minor < 0 {
        return Err(ApplicationError::InvalidInput {
            field: "valueMinor".into(),
            message: "must be zero or greater (integer minor units)".into(),
        });
    }
    let currency_code = patch.currency_code.trim().to_ascii_uppercase();
    if currency_code.len() != 3 || !currency_code.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(ApplicationError::InvalidInput {
            field: "currencyCode".into(),
            message: "must be a three-letter ISO code like USD".into(),
        });
    }
    if let Some(probability) = patch.probability_percent {
        if !(0..=100).contains(&probability) {
            return Err(ApplicationError::InvalidInput {
                field: "probabilityPercent".into(),
                message: "must be between 0 and 100".into(),
            });
        }
    }
    let source =
        match optional_text(patch.source) {
            None => None,
            Some(value) => Some(OpportunitySource::from_database_value(&value).ok_or_else(
                || ApplicationError::InvalidInput {
                    field: "source".into(),
                    message: format!(
                        "unknown source \"{value}\"; expected one of referral, repeat_client, \
                     website, sign, other"
                    ),
                },
            )?),
        };
    Ok(ValidOpportunityFields {
        name: required_text("name", patch.name, 200)?,
        contact_id,
        company_id,
        value: Money {
            value_minor: patch.value_minor,
            currency_code,
        },
        probability_percent: patch.probability_percent,
        expected_close_date: optional_text(patch.expected_close_date),
        source,
        source_label: optional_text(patch.source_label),
        notes: optional_text(patch.notes),
    })
}

/// Activity patch after validation, with parsed enums and trimmed text.
struct ValidActivityFields {
    kind: ActivityKind,
    direction: ActivityDirection,
    occurred_at: String,
    summary: String,
    body: Option<String>,
}

fn validate_activity_patch(patch: ActivityPatch) -> Result<ValidActivityFields, ApplicationError> {
    let kind = ActivityKind::from_database_value(patch.kind.trim()).ok_or_else(|| {
        ApplicationError::InvalidInput {
            field: "kind".into(),
            message: format!(
                "unknown kind \"{}\"; expected one of call, email, text, site_visit, \
                 meeting, note",
                patch.kind
            ),
        }
    })?;
    let direction = match optional_text(patch.direction) {
        None => ActivityDirection::None,
        Some(value) => ActivityDirection::from_database_value(&value).ok_or_else(|| {
            ApplicationError::InvalidInput {
                field: "direction".into(),
                message: format!(
                    "unknown direction \"{value}\"; expected one of inbound, outbound, none"
                ),
            }
        })?,
    };
    // Light rule: notes and on-site touches have no direction; calls, emails,
    // and texts may be inbound, outbound, or none.
    if matches!(
        kind,
        ActivityKind::Note | ActivityKind::SiteVisit | ActivityKind::Meeting
    ) && direction != ActivityDirection::None
    {
        return Err(ApplicationError::InvalidInput {
            field: "direction".into(),
            message: format!("must be none for {} activities", kind.as_database_value()),
        });
    }
    Ok(ValidActivityFields {
        kind,
        direction,
        // User-editable; defaults to now so quick logging stays one field.
        occurred_at: optional_text(patch.occurred_at).unwrap_or_else(now_utc),
        summary: required_text("summary", patch.summary, 500)?,
        body: optional_text(patch.body),
    })
}

/// Task patch after validation, with a parsed optional parent and priority.
struct ValidTaskFields {
    title: String,
    body: Option<String>,
    parent: Option<(ParentType, String)>,
    due_at: Option<String>,
    remind_at: Option<String>,
    priority: TaskPriority,
}

fn validate_task_patch(patch: TaskPatch) -> Result<ValidTaskFields, ApplicationError> {
    let priority = match optional_text(patch.priority) {
        None => TaskPriority::Normal,
        Some(value) => TaskPriority::from_database_value(&value).ok_or_else(|| {
            ApplicationError::InvalidInput {
                field: "priority".into(),
                message: format!("unknown priority \"{value}\"; expected one of low, normal, high"),
            }
        })?,
    };
    Ok(ValidTaskFields {
        title: required_text("title", patch.title, 200)?,
        body: optional_text(patch.body),
        parent: parse_optional_parent(patch.parent_type, patch.parent_id)?,
        due_at: optional_text(patch.due_at),
        remind_at: optional_text(patch.remind_at),
        priority,
    })
}

/// A task's optional parent: type and id together or neither.
fn parse_optional_parent(
    parent_type: Option<String>,
    parent_id: Option<String>,
) -> Result<Option<(ParentType, String)>, ApplicationError> {
    match (optional_text(parent_type), optional_text(parent_id)) {
        (None, None) => Ok(None),
        (Some(parent_type), Some(parent_id)) => Ok(Some((
            parse_parent_type("parentType", &parent_type)?,
            parent_id,
        ))),
        _ => Err(ApplicationError::InvalidInput {
            field: "parentType".into(),
            message: "parentType and parentId must be set together or both left empty".into(),
        }),
    }
}

fn parse_parent_type(field: &str, value: &str) -> Result<ParentType, ApplicationError> {
    ParentType::from_database_value(value.trim()).ok_or_else(|| ApplicationError::InvalidInput {
        field: field.into(),
        message: format!(
            "unknown parent type \"{value}\"; expected one of contact, company, opportunity"
        ),
    })
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

const STAGE_COLUMNS: &str =
    "id, pipeline_id, name, sort_key, kind, created_at, updated_at, version";

const OPPORTUNITY_COLUMNS: &str = "id, name, contact_id, company_id, stage_id, value_minor, \
    currency_code, probability_percent, expected_close_date, source, source_label, \
    lost_reason_id, notes, quote_tool, quote_external_id, quote_label, quote_linked_at, \
    job_tool, job_external_id, job_label, job_linked_at, \
    archived_at, created_at, updated_at, version";

/// Row tuple with the raw kind text, finished into a Stage after parsing.
type StageRow = (Stage, String);

fn stage_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StageRow> {
    let kind_text: String = row.get(4)?;
    Ok((
        Stage {
            id: row.get(0)?,
            pipeline_id: row.get(1)?,
            name: row.get(2)?,
            sort_key: row.get(3)?,
            kind: StageKind::Open, // replaced in finish_stage
            created_at: row.get(5)?,
            updated_at: row.get(6)?,
            version: row.get(7)?,
        },
        kind_text,
    ))
}

/// Parse the stored kind text once the rusqlite row mapping is done.
fn finish_stage((mut stage, kind_text): StageRow) -> Result<Stage, ApplicationError> {
    stage.kind = StageKind::from_database_value(&kind_text).ok_or_else(|| {
        ApplicationError::InvalidStoredData(format!(
            "stage {} has unsupported kind {kind_text}",
            stage.id
        ))
    })?;
    Ok(stage)
}

fn require_stage(transaction: &Transaction<'_>, stage_id: &str) -> Result<Stage, ApplicationError> {
    let row = transaction
        .query_row(
            &format!("SELECT {STAGE_COLUMNS} FROM stages WHERE id = ?1"),
            [stage_id],
            stage_from_row,
        )
        .optional()?
        .ok_or_else(|| ApplicationError::NotFound {
            resource: "stage",
            id: stage_id.into(),
        })?;
    finish_stage(row)
}

/// The pipeline's first open stage — where new opportunities start.
fn first_open_stage(transaction: &Transaction<'_>) -> Result<Stage, ApplicationError> {
    let row = transaction
        .query_row(
            &format!(
                "SELECT {STAGE_COLUMNS} FROM stages WHERE kind = 'open'
                 ORDER BY sort_key, id LIMIT 1"
            ),
            [],
            stage_from_row,
        )
        .optional()?
        .ok_or_else(|| ApplicationError::InvalidStoredData("pipeline has no open stage".into()))?;
    finish_stage(row)
}

/// An opportunity's linked contact must exist (any archive state).
fn require_linked_contact(
    transaction: &Transaction<'_>,
    contact_id: Option<&str>,
) -> Result<(), ApplicationError> {
    let Some(contact_id) = contact_id else {
        return Ok(());
    };
    let exists: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM contacts WHERE id = ?1)",
        [contact_id],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(ApplicationError::NotFound {
            resource: "contact",
            id: contact_id.into(),
        });
    }
    Ok(())
}

/// A referenced lost reason must exist (active or not).
fn require_lost_reason(
    transaction: &Transaction<'_>,
    lost_reason_id: &str,
) -> Result<(), ApplicationError> {
    let exists: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM lost_reasons WHERE id = ?1)",
        [lost_reason_id],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(ApplicationError::NotFound {
            resource: "lost_reason",
            id: lost_reason_id.into(),
        });
    }
    Ok(())
}

/// Row tuple with raw enum text, finished into an Opportunity after parsing.
type OpportunityRow = (Opportunity, Option<String>);

fn opportunity_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<OpportunityRow> {
    let source_text: Option<String> = row.get(9)?;
    Ok((
        Opportunity {
            id: row.get(0)?,
            name: row.get(1)?,
            contact_id: row.get(2)?,
            company_id: row.get(3)?,
            stage_id: row.get(4)?,
            value: Money {
                value_minor: row.get(5)?,
                currency_code: row.get(6)?,
            },
            probability_percent: row.get(7)?,
            expected_close_date: row.get(8)?,
            source: None, // replaced in finish_opportunity
            source_label: row.get(10)?,
            lost_reason_id: row.get(11)?,
            notes: row.get(12)?,
            quote_ref: handoff_ref_from_columns(
                row.get(13)?,
                row.get(14)?,
                row.get(15)?,
                row.get(16)?,
            ),
            job_ref: handoff_ref_from_columns(
                row.get(17)?,
                row.get(18)?,
                row.get(19)?,
                row.get(20)?,
            ),
            archived_at: row.get(21)?,
            created_at: row.get(22)?,
            updated_at: row.get(23)?,
            version: row.get(24)?,
        },
        source_text,
    ))
}

/// Assemble an optional hand-off ref from its four columns; a ref exists only
/// when tool, external id, and linked timestamp are all present.
fn handoff_ref_from_columns(
    tool: Option<String>,
    external_id: Option<String>,
    label: Option<String>,
    linked_at: Option<String>,
) -> Option<HandoffRef> {
    match (tool, external_id, linked_at) {
        (Some(tool), Some(external_id), Some(linked_at)) => Some(HandoffRef {
            tool,
            external_id,
            label,
            linked_at,
        }),
        _ => None,
    }
}

/// Parse the stored source text once the rusqlite row mapping is done.
fn finish_opportunity(
    (mut opportunity, source_text): OpportunityRow,
) -> Result<Opportunity, ApplicationError> {
    opportunity.source = match source_text {
        None => None,
        Some(source_text) => Some(
            OpportunitySource::from_database_value(&source_text).ok_or_else(|| {
                ApplicationError::InvalidStoredData(format!(
                    "opportunity {} has unsupported source {source_text}",
                    opportunity.id
                ))
            })?,
        ),
    };
    Ok(opportunity)
}

fn require_opportunity(
    transaction: &Transaction<'_>,
    opportunity_id: &str,
) -> Result<Opportunity, ApplicationError> {
    let row = transaction
        .query_row(
            &format!("SELECT {OPPORTUNITY_COLUMNS} FROM opportunities WHERE id = ?1"),
            [opportunity_id],
            opportunity_from_row,
        )
        .optional()?
        .ok_or_else(|| ApplicationError::NotFound {
            resource: "opportunity",
            id: opportunity_id.into(),
        })?;
    finish_opportunity(row)
}

/// Append one stage_history row; part of the same mutation transaction.
fn insert_stage_history(
    transaction: &Transaction<'_>,
    opportunity_id: &str,
    from_stage_id: Option<&str>,
    to_stage_id: &str,
    actor: Actor,
    lost_reason_id: Option<&str>,
) -> Result<(), ApplicationError> {
    transaction.execute(
        "INSERT INTO stage_history
            (id, opportunity_id, from_stage_id, to_stage_id, actor, lost_reason_id, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            new_id(),
            opportunity_id,
            from_stage_id,
            to_stage_id,
            actor.as_database_value(),
            lost_reason_id,
            now_utc(),
        ],
    )?;
    Ok(())
}

fn load_stage_history(
    connection: &rusqlite::Connection,
    opportunity_id: &str,
) -> Result<Vec<StageHistoryEntry>, ApplicationError> {
    let mut statement = connection.prepare(
        "SELECT id, opportunity_id, from_stage_id, to_stage_id, actor, lost_reason_id, created_at
         FROM stage_history WHERE opportunity_id = ?1 ORDER BY created_at, id",
    )?;
    let rows = statement
        .query_map([opportunity_id], |row| {
            Ok((
                StageHistoryEntry {
                    id: row.get(0)?,
                    opportunity_id: row.get(1)?,
                    from_stage_id: row.get(2)?,
                    to_stage_id: row.get(3)?,
                    actor: Actor::User, // replaced below
                    lost_reason_id: row.get(5)?,
                    created_at: row.get(6)?,
                },
                row.get::<_, String>(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(mut entry, actor_text)| {
            entry.actor = Actor::from_database_value(&actor_text).ok_or_else(|| {
                ApplicationError::InvalidStoredData(format!(
                    "stage_history {} has unsupported actor {actor_text}",
                    entry.id
                ))
            })?;
            Ok(entry)
        })
        .collect()
}

const ACTIVITY_COLUMNS: &str = "id, parent_type, parent_id, kind, direction, occurred_at, \
    summary, body, actor, created_at, updated_at, version";

/// Row tuple with raw enum texts, finished into an Activity after parsing.
type ActivityRow = (Activity, String, String, String, String);

fn activity_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ActivityRow> {
    let parent_type_text: String = row.get(1)?;
    let kind_text: String = row.get(3)?;
    let direction_text: String = row.get(4)?;
    let actor_text: String = row.get(8)?;
    Ok((
        Activity {
            id: row.get(0)?,
            parent_type: ParentType::Contact, // replaced in finish_activity
            parent_id: row.get(2)?,
            kind: ActivityKind::Note, // replaced in finish_activity
            direction: ActivityDirection::None, // replaced in finish_activity
            occurred_at: row.get(5)?,
            summary: row.get(6)?,
            body: row.get(7)?,
            actor: Actor::User, // replaced in finish_activity
            created_at: row.get(9)?,
            updated_at: row.get(10)?,
            version: row.get(11)?,
        },
        parent_type_text,
        kind_text,
        direction_text,
        actor_text,
    ))
}

/// Parse stored enum texts once the rusqlite row mapping is done.
fn finish_activity(
    (mut activity, parent_type_text, kind_text, direction_text, actor_text): ActivityRow,
) -> Result<Activity, ApplicationError> {
    let invalid = |what: &str, value: &str| {
        ApplicationError::InvalidStoredData(format!(
            "activity {} has unsupported {what} {value}",
            activity.id
        ))
    };
    activity.parent_type = ParentType::from_database_value(&parent_type_text)
        .ok_or_else(|| invalid("parent type", &parent_type_text))?;
    activity.kind =
        ActivityKind::from_database_value(&kind_text).ok_or_else(|| invalid("kind", &kind_text))?;
    activity.direction = ActivityDirection::from_database_value(&direction_text)
        .ok_or_else(|| invalid("direction", &direction_text))?;
    activity.actor =
        Actor::from_database_value(&actor_text).ok_or_else(|| invalid("actor", &actor_text))?;
    Ok(activity)
}

fn require_activity(
    transaction: &Transaction<'_>,
    activity_id: &str,
) -> Result<Activity, ApplicationError> {
    let row = transaction
        .query_row(
            &format!("SELECT {ACTIVITY_COLUMNS} FROM activities WHERE id = ?1"),
            [activity_id],
            activity_from_row,
        )
        .optional()?
        .ok_or_else(|| ApplicationError::NotFound {
            resource: "activity",
            id: activity_id.into(),
        })?;
    finish_activity(row)
}

const TASK_COLUMNS: &str = "id, title, body, parent_type, parent_id, due_at, remind_at, \
    priority, status, completed_at, created_at, updated_at, version";

/// Row tuple with raw enum texts, finished into a Task after parsing.
type TaskRow = (Task, Option<String>, String, String);

fn task_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskRow> {
    let parent_type_text: Option<String> = row.get(3)?;
    let priority_text: String = row.get(7)?;
    let status_text: String = row.get(8)?;
    Ok((
        Task {
            id: row.get(0)?,
            title: row.get(1)?,
            body: row.get(2)?,
            parent_type: None, // replaced in finish_task
            parent_id: row.get(4)?,
            due_at: row.get(5)?,
            remind_at: row.get(6)?,
            priority: TaskPriority::Normal, // replaced in finish_task
            status: TaskStatus::Open,       // replaced in finish_task
            completed_at: row.get(9)?,
            created_at: row.get(10)?,
            updated_at: row.get(11)?,
            version: row.get(12)?,
        },
        parent_type_text,
        priority_text,
        status_text,
    ))
}

/// Parse stored enum texts once the rusqlite row mapping is done.
fn finish_task(
    (mut task, parent_type_text, priority_text, status_text): TaskRow,
) -> Result<Task, ApplicationError> {
    let invalid = |what: &str, value: &str| {
        ApplicationError::InvalidStoredData(format!(
            "task {} has unsupported {what} {value}",
            task.id
        ))
    };
    task.parent_type = match &parent_type_text {
        None => None,
        Some(text) => Some(
            ParentType::from_database_value(text).ok_or_else(|| invalid("parent type", text))?,
        ),
    };
    task.priority = TaskPriority::from_database_value(&priority_text)
        .ok_or_else(|| invalid("priority", &priority_text))?;
    task.status = TaskStatus::from_database_value(&status_text)
        .ok_or_else(|| invalid("status", &status_text))?;
    Ok(task)
}

fn require_task(transaction: &Transaction<'_>, task_id: &str) -> Result<Task, ApplicationError> {
    let row = transaction
        .query_row(
            &format!("SELECT {TASK_COLUMNS} FROM tasks WHERE id = ?1"),
            [task_id],
            task_from_row,
        )
        .optional()?
        .ok_or_else(|| ApplicationError::NotFound {
            resource: "task",
            id: task_id.into(),
        })?;
    finish_task(row)
}

/// An activity's polymorphic parent must exist in its table (any archive
/// state) — the schema has no FK on parent_id, so this is the guard.
fn require_activity_parent(
    connection: &rusqlite::Connection,
    parent_type: ParentType,
    parent_id: &str,
) -> Result<(), ApplicationError> {
    let sql = match parent_type {
        ParentType::Contact => "SELECT EXISTS(SELECT 1 FROM contacts WHERE id = ?1)",
        ParentType::Company => "SELECT EXISTS(SELECT 1 FROM companies WHERE id = ?1)",
        ParentType::Opportunity => "SELECT EXISTS(SELECT 1 FROM opportunities WHERE id = ?1)",
    };
    let exists: bool = connection.query_row(sql, [parent_id], |row| row.get(0))?;
    if !exists {
        return Err(ApplicationError::NotFound {
            resource: parent_type.as_database_value(),
            id: parent_id.into(),
        });
    }
    Ok(())
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
