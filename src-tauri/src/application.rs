//! Application use-cases for companies and contacts. Every mutation runs in
//! one immediate transaction, checks the expected record version, bumps the
//! version, and writes a command_log row before committing.

use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Deserializer, Serialize};

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
// Saved views (versioned filter definitions)
// ---------------------------------------------------------------------------

/// A saved view applies only to one of the list surfaces implemented in v1.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SavedViewEntityType {
    Contact,
    Company,
    Opportunity,
}

impl SavedViewEntityType {
    fn as_database_value(&self) -> &'static str {
        match self {
            Self::Contact => "contact",
            Self::Company => "company",
            Self::Opportunity => "opportunity",
        }
    }

    fn parse(value: &str) -> Result<Self, ApplicationError> {
        match value {
            "contact" => Ok(Self::Contact),
            "company" => Ok(Self::Company),
            "opportunity" => Ok(Self::Opportunity),
            _ => Err(ApplicationError::InvalidStoredData(format!(
                "saved view has unsupported entity type {value:?}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SavedViewSortDirection {
    Ascending,
    Descending,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SavedViewFilter {
    pub include_archived: bool,
    pub tag_ids_all: Vec<String>,
    pub custom_fields: Vec<SavedViewCustomFieldPredicate>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SavedViewCustomFieldPredicate {
    pub definition_id: String,
    pub field_type: String,
    pub operator: String,
    pub value: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SavedViewSort {
    pub field: String,
    pub direction: SavedViewSortDirection,
}

/// Persisted filter/sort definition. `schemaVersion` is deliberately stored
/// inside the JSON so future schema changes can be rejected without changing
/// the saved-view table or losing the original bytes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SavedViewDefinition {
    pub schema_version: i64,
    pub filter: SavedViewFilter,
    pub sort: SavedViewSort,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedView {
    pub id: String,
    pub name: String,
    pub entity_type: SavedViewEntityType,
    pub definition: SavedViewDefinition,
    pub sort_key: i64,
    pub created_at: String,
    pub updated_at: String,
    pub version: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSavedViewRequest {
    #[serde(default)]
    pub actor: Actor,
    pub name: String,
    pub entity_type: SavedViewEntityType,
    pub definition: SavedViewDefinition,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSavedViewRequest {
    #[serde(default)]
    pub actor: Actor,
    pub saved_view_id: String,
    pub expected_version: i64,
    pub name: String,
    pub definition: SavedViewDefinition,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteSavedViewRequest {
    #[serde(default)]
    pub actor: Actor,
    pub saved_view_id: String,
    pub expected_version: i64,
}

const SAVED_VIEW_SCHEMA_VERSION: i64 = 2;
const MAX_SAVED_VIEWS_PER_SURFACE: i64 = 50;
const MAX_TAGS: i64 = 100;
const MAX_FIELD_DEFS: i64 = 50;
const MAX_FIELD_OPTIONS: i64 = 50;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tag {
    pub id: String,
    pub label: String,
    pub color_role: Option<String>,
    pub archived_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub version: i64,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateTagRequest {
    #[serde(default)]
    pub actor: Actor,
    pub label: String,
    pub color_role: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateTagRequest {
    #[serde(default)]
    pub actor: Actor,
    pub tag_id: String,
    pub expected_version: i64,
    pub label: String,
    pub color_role: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TagArchiveRequest {
    #[serde(default)]
    pub actor: Actor,
    pub tag_id: String,
    pub expected_version: i64,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CustomFieldOptionInput {
    pub id: Option<String>,
    pub label: String,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomFieldOption {
    pub id: String,
    pub definition_id: String,
    pub label: String,
    pub sort_key: i64,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomFieldDef {
    pub id: String,
    pub entity_type: SavedViewEntityType,
    pub label: String,
    pub field_type: String,
    pub sort_key: i64,
    pub archived_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub version: i64,
    pub options: Vec<CustomFieldOption>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateCustomFieldDefRequest {
    #[serde(default)]
    pub actor: Actor,
    pub entity_type: SavedViewEntityType,
    pub label: String,
    pub field_type: String,
    #[serde(default)]
    pub options: Vec<CustomFieldOptionInput>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateCustomFieldDefRequest {
    #[serde(default)]
    pub actor: Actor,
    pub definition_id: String,
    pub expected_version: i64,
    pub label: String,
    pub sort_key: i64,
    pub options: Vec<CustomFieldOptionInput>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CustomFieldDefArchiveRequest {
    #[serde(default)]
    pub actor: Actor,
    pub definition_id: String,
    pub expected_version: i64,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CustomFieldValueInput {
    pub definition_id: String,
    #[serde(deserialize_with = "required_option")]
    pub text_value: Option<String>,
    #[serde(deserialize_with = "required_option")]
    pub number_value: Option<f64>,
    #[serde(deserialize_with = "required_option")]
    pub date_value: Option<String>,
    #[serde(deserialize_with = "required_option")]
    pub option_id: Option<String>,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomFieldValue {
    pub id: String,
    pub definition_id: String,
    pub entity_type: SavedViewEntityType,
    pub record_id: String,
    pub text_value: Option<String>,
    pub number_value: Option<f64>,
    pub date_value: Option<String>,
    pub option_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordMetadata {
    pub tag_ids: Vec<String>,
    pub values: Vec<CustomFieldValue>,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetRecordMetadataRequest {
    #[serde(default)]
    pub actor: Actor,
    pub entity_type: SavedViewEntityType,
    pub record_id: String,
    pub expected_version: i64,
    pub tag_ids: Vec<String>,
    pub values: Vec<CustomFieldValueInput>,
}

fn required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

pub fn list_tags(storage: &Storage, include_archived: bool) -> Result<Vec<Tag>, ApplicationError> {
    let mut statement = storage.connection().prepare("SELECT id,label,color_role,archived_at,created_at,updated_at,version FROM tags WHERE ?1 OR archived_at IS NULL ORDER BY label COLLATE NOCASE,id")?;
    let tags = statement
        .query_map([include_archived], tag_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(ApplicationError::from)?;
    Ok(tags)
}
pub fn create_tag(
    storage: &mut Storage,
    request: CreateTagRequest,
) -> Result<Tag, ApplicationError> {
    let label = required_text("label", request.label, 80)?;
    validate_color_role(request.color_role.as_deref())?;
    let transaction = immediate(storage)?;
    let count: i64 = transaction.query_row("SELECT COUNT(*) FROM tags", [], |r| r.get(0))?;
    if count >= MAX_TAGS {
        return Err(limit_error("tag_limit_reached", "label", MAX_TAGS));
    }
    require_tag_label_available(&transaction, &label, None)?;
    let id = new_id();
    let now = now_utc();
    transaction.execute("INSERT INTO tags (id,label,color_role,created_at,updated_at,version) VALUES (?1,?2,?3,?4,?4,1)", params![id,label,request.color_role,now])?;
    log_command(&transaction, request.actor, "tag", &id, "created tag")?;
    let tag = require_tag(&transaction, &id)?;
    transaction.commit()?;
    Ok(tag)
}
pub fn update_tag(
    storage: &mut Storage,
    request: UpdateTagRequest,
) -> Result<Tag, ApplicationError> {
    let label = required_text("label", request.label, 80)?;
    validate_color_role(request.color_role.as_deref())?;
    let transaction = immediate(storage)?;
    let existing = require_tag(&transaction, &request.tag_id)?;
    check_version(
        "tag",
        &request.tag_id,
        request.expected_version,
        existing.version,
    )?;
    require_tag_label_available(&transaction, &label, Some(&request.tag_id))?;
    transaction.execute(
        "UPDATE tags SET label=?2,color_role=?3,updated_at=?4,version=?5 WHERE id=?1",
        params![
            request.tag_id,
            label,
            request.color_role,
            now_utc(),
            existing.version + 1
        ],
    )?;
    log_command(
        &transaction,
        request.actor,
        "tag",
        &request.tag_id,
        "updated tag",
    )?;
    let tag = require_tag(&transaction, &request.tag_id)?;
    transaction.commit()?;
    Ok(tag)
}
pub fn archive_tag(
    storage: &mut Storage,
    request: TagArchiveRequest,
) -> Result<Tag, ApplicationError> {
    set_tag_archive(storage, request, true)
}
pub fn unarchive_tag(
    storage: &mut Storage,
    request: TagArchiveRequest,
) -> Result<Tag, ApplicationError> {
    set_tag_archive(storage, request, false)
}

pub fn list_custom_field_defs(
    storage: &Storage,
    entity_type: SavedViewEntityType,
    include_archived: bool,
) -> Result<Vec<CustomFieldDef>, ApplicationError> {
    let mut statement=storage.connection().prepare("SELECT id,entity_type,label,field_type,sort_key,archived_at,created_at,updated_at,version FROM custom_field_defs WHERE entity_type=?1 AND (?2 OR archived_at IS NULL) ORDER BY sort_key,id")?;
    let rows = statement
        .query_map(
            params![entity_type.as_database_value(), include_archived],
            custom_field_def_from_row,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|d| finish_custom_field_def(storage.connection(), d))
        .collect()
}
pub fn create_custom_field_def(
    storage: &mut Storage,
    request: CreateCustomFieldDefRequest,
) -> Result<CustomFieldDef, ApplicationError> {
    let label = required_text("label", request.label, 120)?;
    validate_field_type(&request.field_type)?;
    validate_option_inputs(&request.field_type, &request.options)?;
    let transaction = immediate(storage)?;
    let count: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM custom_field_defs WHERE entity_type=?1",
        [request.entity_type.as_database_value()],
        |r| r.get(0),
    )?;
    if count >= MAX_FIELD_DEFS {
        return Err(limit_error(
            "custom_field_def_limit_reached",
            "label",
            MAX_FIELD_DEFS,
        ));
    }
    require_custom_field_label_available(&transaction, &request.entity_type, &label, None)?;
    let sort_key: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(sort_key),-1)+1 FROM custom_field_defs WHERE entity_type=?1",
        [request.entity_type.as_database_value()],
        |r| r.get(0),
    )?;
    let id = new_id();
    let now = now_utc();
    transaction.execute("INSERT INTO custom_field_defs (id,entity_type,label,field_type,sort_key,created_at,updated_at,version) VALUES (?1,?2,?3,?4,?5,?6,?6,1)",params![id,request.entity_type.as_database_value(),label,request.field_type,sort_key,now])?;
    replace_options(&transaction, &id, &request.options)?;
    log_command(
        &transaction,
        request.actor,
        "custom_field_def",
        &id,
        "created custom field",
    )?;
    let def = require_custom_field_def(&transaction, &id)?;
    transaction.commit()?;
    Ok(def)
}
pub fn update_custom_field_def(
    storage: &mut Storage,
    request: UpdateCustomFieldDefRequest,
) -> Result<CustomFieldDef, ApplicationError> {
    let label = required_text("label", request.label, 120)?;
    if request.sort_key < 0 {
        return Err(ApplicationError::InvalidInput {
            field: "sortKey".into(),
            message: "must be zero or greater".into(),
        });
    }
    let transaction = immediate(storage)?;
    let existing = require_custom_field_def(&transaction, &request.definition_id)?;
    check_version(
        "custom_field_def",
        &existing.id,
        request.expected_version,
        existing.version,
    )?;
    validate_option_inputs(&existing.field_type, &request.options)?;
    require_custom_field_label_available(
        &transaction,
        &existing.entity_type,
        &label,
        Some(&existing.id),
    )?;
    replace_options(&transaction, &existing.id, &request.options)?;
    transaction.execute(
        "UPDATE custom_field_defs SET label=?2,sort_key=?3,updated_at=?4,version=?5 WHERE id=?1",
        params![
            existing.id,
            label,
            request.sort_key,
            now_utc(),
            existing.version + 1
        ],
    )?;
    log_command(
        &transaction,
        request.actor,
        "custom_field_def",
        &existing.id,
        "updated custom field",
    )?;
    let def = require_custom_field_def(&transaction, &existing.id)?;
    transaction.commit()?;
    Ok(def)
}
pub fn archive_custom_field_def(
    storage: &mut Storage,
    request: CustomFieldDefArchiveRequest,
) -> Result<CustomFieldDef, ApplicationError> {
    set_def_archive(storage, request, true)
}
pub fn unarchive_custom_field_def(
    storage: &mut Storage,
    request: CustomFieldDefArchiveRequest,
) -> Result<CustomFieldDef, ApplicationError> {
    set_def_archive(storage, request, false)
}

pub fn get_record_metadata(
    storage: &Storage,
    entity_type: SavedViewEntityType,
    record_id: &str,
) -> Result<RecordMetadata, ApplicationError> {
    ensure_owner(storage.connection(), &entity_type, record_id)?;
    load_metadata(storage.connection(), &entity_type, record_id)
}
pub fn set_record_metadata(
    storage: &mut Storage,
    request: SetRecordMetadataRequest,
) -> Result<RecordMetadata, ApplicationError> {
    validate_metadata_request(&request)?;
    let transaction = immediate(storage)?;
    let current = owner_version(&transaction, &request.entity_type, &request.record_id)?;
    check_version(
        request.entity_type.as_database_value(),
        &request.record_id,
        request.expected_version,
        current,
    )?;
    let existing = load_metadata(&transaction, &request.entity_type, &request.record_id)?;
    validate_metadata_references(&transaction, &request, &existing)?;
    if metadata_equal(&existing, &request) {
        transaction.commit()?;
        return Ok(existing);
    }
    transaction.execute(
        "DELETE FROM record_tags WHERE entity_type=?1 AND record_id=?2",
        params![request.entity_type.as_database_value(), request.record_id],
    )?;
    transaction.execute(
        "DELETE FROM custom_field_values WHERE entity_type=?1 AND record_id=?2",
        params![request.entity_type.as_database_value(), request.record_id],
    )?;
    let now = now_utc();
    for tag_id in &request.tag_ids {
        transaction.execute("INSERT INTO record_tags (tag_id,entity_type,record_id,created_at) VALUES (?1,?2,?3,?4)",params![tag_id,request.entity_type.as_database_value(),request.record_id,now])?;
    }
    for value in &request.values {
        transaction.execute("INSERT INTO custom_field_values (id,definition_id,entity_type,record_id,text_value,number_value,date_value,option_id,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?9)",params![new_id(),value.definition_id,request.entity_type.as_database_value(),request.record_id,value.text_value,value.number_value,value.date_value,value.option_id,now])?;
    }
    bump_owner(
        &transaction,
        &request.entity_type,
        &request.record_id,
        current + 1,
        &now,
    )?;
    log_command(
        &transaction,
        request.actor,
        request.entity_type.as_database_value(),
        &request.record_id,
        "updated record metadata",
    )?;
    let metadata = load_metadata(&transaction, &request.entity_type, &request.record_id)?;
    transaction.commit()?;
    Ok(metadata)
}

pub fn match_saved_view(
    storage: &Storage,
    entity_type: SavedViewEntityType,
    definition: SavedViewDefinition,
) -> Result<Vec<String>, ApplicationError> {
    let definition = validate_saved_view_definition(entity_type.clone(), definition)?;
    validate_saved_view_references(storage.connection(), &entity_type, &definition)?;
    let table = entity_type.as_database_value();
    let sql = format!(
        "SELECT id FROM {} WHERE {} ORDER BY id",
        match table {
            "contact" => "contacts",
            "company" => "companies",
            _ => "opportunities",
        },
        if definition.filter.include_archived {
            "1=1"
        } else {
            "archived_at IS NULL"
        }
    );
    let ids = storage
        .connection()
        .prepare(&sql)?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    let mut matched = Vec::new();
    for id in ids {
        let mut keep = true;
        for tag in &definition.filter.tag_ids_all {
            let assigned: bool = storage.connection().query_row(
                "SELECT EXISTS(SELECT 1 FROM record_tags WHERE tag_id=?1 AND entity_type=?2 AND record_id=?3)",
                params![tag, table, &id],
                |row| row.get(0),
            )?;
            if !assigned {
                keep = false;
                break;
            }
        }
        if keep {
            for predicate in &definition.filter.custom_fields {
                if !matches_predicate(storage.connection(), table, &id, predicate)? {
                    keep = false;
                    break;
                }
            }
        }
        if keep {
            matched.push(id);
        }
    }
    Ok(matched)
}

/// Return views for one list surface in their durable, deterministic order.
/// Invalid stored definitions are reported, never rewritten or skipped.
pub fn list_saved_views(
    storage: &Storage,
    entity_type: SavedViewEntityType,
) -> Result<Vec<SavedView>, ApplicationError> {
    let mut statement = storage.connection().prepare(
        "SELECT id, name, entity_type, definition_json, sort_key, created_at, updated_at, version
         FROM saved_views WHERE entity_type = ?1 ORDER BY sort_key, id",
    )?;
    let saved_views = statement
        .query_map([entity_type.as_database_value()], saved_view_raw_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    saved_views.into_iter().map(finish_saved_view).collect()
}

/// Create a bounded, typed saved definition and append it to its surface.
pub fn create_saved_view(
    storage: &mut Storage,
    request: CreateSavedViewRequest,
) -> Result<SavedView, ApplicationError> {
    let name = validate_saved_view_name(request.name)?;
    let definition =
        validate_saved_view_definition(request.entity_type.clone(), request.definition)?;
    let definition_json = serde_json::to_string(&definition)
        .map_err(|error| ApplicationError::InvalidStoredData(error.to_string()))?;
    let id = new_id();
    let now = now_utc();
    let transaction = immediate(storage)?;
    validate_saved_view_references(&transaction, &request.entity_type, &definition)?;
    let count: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM saved_views WHERE entity_type = ?1",
        [request.entity_type.as_database_value()],
        |row| row.get(0),
    )?;
    if count >= MAX_SAVED_VIEWS_PER_SURFACE {
        return Err(ApplicationError::ValidationFailed {
            code: "saved_view_limit_reached",
            field: "name".into(),
            message: format!(
                "a list surface may have at most {MAX_SAVED_VIEWS_PER_SURFACE} saved views"
            ),
        });
    }
    require_saved_view_name_available(
        &transaction,
        request.entity_type.as_database_value(),
        &name,
        None,
    )?;
    let sort_key: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(sort_key), -1) + 1 FROM saved_views WHERE entity_type = ?1",
        [request.entity_type.as_database_value()],
        |row| row.get(0),
    )?;
    transaction.execute(
        "INSERT INTO saved_views
         (id, name, entity_type, definition_json, sort_key, created_at, updated_at, version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, 1)",
        params![
            id,
            name,
            request.entity_type.as_database_value(),
            definition_json,
            sort_key,
            now
        ],
    )?;
    log_command(
        &transaction,
        request.actor,
        "saved_view",
        &id,
        "created saved view",
    )?;
    let saved_view = require_saved_view(&transaction, &id)?;
    transaction.commit()?;
    Ok(saved_view)
}

/// Replace a view definition/name after an optimistic-version check.
pub fn update_saved_view(
    storage: &mut Storage,
    request: UpdateSavedViewRequest,
) -> Result<SavedView, ApplicationError> {
    let name = validate_saved_view_name(request.name)?;
    let transaction = immediate(storage)?;
    let existing = require_saved_view(&transaction, &request.saved_view_id)?;
    check_version(
        "saved_view",
        &existing.id,
        request.expected_version,
        existing.version,
    )?;
    let definition =
        validate_saved_view_definition(existing.entity_type.clone(), request.definition)?;
    validate_saved_view_references(&transaction, &existing.entity_type, &definition)?;
    let definition_json = serde_json::to_string(&definition)
        .map_err(|error| ApplicationError::InvalidStoredData(error.to_string()))?;
    require_saved_view_name_available(
        &transaction,
        existing.entity_type.as_database_value(),
        &name,
        Some(&existing.id),
    )?;
    transaction.execute(
        "UPDATE saved_views SET name = ?2, definition_json = ?3, updated_at = ?4, version = ?5
         WHERE id = ?1",
        params![
            existing.id,
            name,
            definition_json,
            now_utc(),
            existing.version + 1
        ],
    )?;
    log_command(
        &transaction,
        request.actor,
        "saved_view",
        &existing.id,
        "updated saved view",
    )?;
    let saved_view = require_saved_view(&transaction, &existing.id)?;
    transaction.commit()?;
    Ok(saved_view)
}

/// Delete a saved view after an optimistic-version check. This is a deliberate
/// permanent deletion of configuration, unlike archiving canonical CRM data.
pub fn delete_saved_view(
    storage: &mut Storage,
    request: DeleteSavedViewRequest,
) -> Result<(), ApplicationError> {
    let transaction = immediate(storage)?;
    let existing = require_saved_view(&transaction, &request.saved_view_id)?;
    check_version(
        "saved_view",
        &existing.id,
        request.expected_version,
        existing.version,
    )?;
    transaction.execute("DELETE FROM saved_views WHERE id = ?1", [&existing.id])?;
    log_command(
        &transaction,
        request.actor,
        "saved_view",
        &existing.id,
        "deleted saved view",
    )?;
    transaction.commit()?;
    Ok(())
}

/// One bounded FTS result. `entity_type` is one of contact, company,
/// opportunity, or activity; the canonical record is loaded separately when
/// callers need its full shape.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub entity_type: String,
    pub entity_id: String,
    pub title: String,
    pub parent_type: Option<String>,
    pub parent_id: Option<String>,
}

/// Ordered app-settings entry for a record the user successfully opened from
/// global search. Activities are intentionally excluded: the UI resolves
/// those to their canonical parent before recording a recent.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct RecentRecord {
    entity_type: String,
    entity_id: String,
}

const NAVIGATION_RECENTS_KEY: &str = "navigation.recents.v1";
const MAX_NAVIGATION_RECENTS: usize = 12;

/// Search local CRM content. Empty/punctuation-only input intentionally has no
/// results; FTS syntax is never passed through from the caller.
pub fn search_records(
    storage: &Storage,
    query: String,
    entity_types: Option<Vec<String>>,
    limit: Option<usize>,
) -> Result<Vec<SearchResult>, ApplicationError> {
    let query = bounded_fts_query(&query);
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let entity_types = entity_types.unwrap_or_else(|| {
        vec![
            "contact".into(),
            "company".into(),
            "opportunity".into(),
            "activity".into(),
        ]
    });
    if entity_types.is_empty() {
        return Ok(Vec::new());
    }
    if entity_types.len() > 4 {
        return Err(ApplicationError::InvalidInput {
            field: "entityTypes".into(),
            message: "must contain at most four values".into(),
        });
    }
    let mut unique_entity_types = Vec::with_capacity(entity_types.len());
    for entity_type in entity_types {
        if !matches!(
            entity_type.as_str(),
            "contact" | "company" | "opportunity" | "activity"
        ) {
            return Err(ApplicationError::InvalidInput {
                field: "entityTypes".into(),
                message: "must contain only contact, company, opportunity, or activity".into(),
            });
        }
        if !unique_entity_types.contains(&entity_type) {
            unique_entity_types.push(entity_type);
        }
    }
    let entity_types = unique_entity_types;
    let limit = limit.unwrap_or(25).clamp(1, 50) as i64;
    let placeholders = (1..=entity_types.len())
        .map(|index| format!("?{}", index + 1))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT search_index.entity_type, search_index.entity_id, search_index.title, \
                activities.parent_type, activities.parent_id FROM search_index \
         LEFT JOIN activities ON search_index.entity_type = 'activity' \
             AND activities.id = search_index.entity_id \
         WHERE search_index MATCH ?1 AND entity_type IN ({placeholders}) \
         ORDER BY bm25(search_index), entity_type, entity_id LIMIT ?{}",
        entity_types.len() + 2
    );
    let mut values: Vec<&dyn rusqlite::ToSql> = vec![&query];
    for entity_type in &entity_types {
        values.push(entity_type);
    }
    values.push(&limit);
    let mut statement = storage.connection().prepare(&sql)?;
    let results = statement
        .query_map(rusqlite::params_from_iter(values), |row| {
            Ok(SearchResult {
                entity_type: row.get(0)?,
                entity_id: row.get(1)?,
                title: row.get(2)?,
                parent_type: row.get(3)?,
                parent_id: row.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into);
    results
}

/// Load the ordered global-search recents projection. Invalid, missing, and
/// archived records are not surfaced; the stored setting remains a durable
/// ordered history and is compacted the next time a recent is recorded.
pub fn list_recent_records(storage: &Storage) -> Result<Vec<SearchResult>, ApplicationError> {
    let entries = read_navigation_recents(storage.connection())?;
    entries
        .into_iter()
        .filter_map(
            |entry| match active_navigation_result(storage.connection(), &entry) {
                Ok(Some(result)) => Some(Ok(result)),
                Ok(None) => None,
                Err(error) => Some(Err(error)),
            },
        )
        .collect()
}

/// Persist a canonical record as the most-recent global-search destination.
/// The target must still be active, which prevents callers from creating a
/// stale navigation projection even if they accidentally record too early.
pub fn record_recent(
    storage: &mut Storage,
    entity_type: String,
    entity_id: String,
) -> Result<(), ApplicationError> {
    let entry = RecentRecord {
        entity_type,
        entity_id,
    };
    if !matches!(
        entry.entity_type.as_str(),
        "contact" | "company" | "opportunity"
    ) {
        return Err(ApplicationError::InvalidInput {
            field: "entityType".into(),
            message: "must be contact, company, or opportunity".into(),
        });
    }
    if entry.entity_id.trim().is_empty() {
        return Err(ApplicationError::InvalidInput {
            field: "entityId".into(),
            message: "must not be empty".into(),
        });
    }

    let transaction = immediate(storage)?;
    if active_navigation_result(&transaction, &entry)?.is_none() {
        return Err(ApplicationError::NotFound {
            resource: navigation_resource(&entry.entity_type),
            id: entry.entity_id,
        });
    }
    let entries = read_navigation_recents(&transaction)?;
    let mut active_entries = Vec::with_capacity(entries.len() + 1);
    for existing in entries {
        if existing != entry
            && !active_entries.iter().any(|record| record == &existing)
            && active_navigation_result(&transaction, &existing)?.is_some()
        {
            active_entries.push(existing);
        }
    }
    active_entries.insert(0, entry);
    active_entries.truncate(MAX_NAVIGATION_RECENTS);
    let value = serde_json::to_string(&active_entries)
        .map_err(|error| ApplicationError::InvalidStoredData(error.to_string()))?;
    transaction.execute(
        "INSERT INTO app_settings (key, value) VALUES (?1, ?2)\n         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![NAVIGATION_RECENTS_KEY, value],
    )?;
    transaction.commit()?;
    Ok(())
}

/// Existing favorite contacts provide the second empty-search projection.
/// Favorites are deliberately contact-only in v1.
pub fn list_favorite_contacts(storage: &Storage) -> Result<Vec<SearchResult>, ApplicationError> {
    let mut statement = storage.connection().prepare(
        "SELECT id, display_name FROM contacts\n         WHERE favorite = 1 AND archived_at IS NULL\n         ORDER BY display_name, id",
    )?;
    let contacts = statement
        .query_map([], |row| {
            Ok(SearchResult {
                entity_type: "contact".into(),
                entity_id: row.get(0)?,
                title: row.get(1)?,
                parent_type: None,
                parent_id: None,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(contacts)
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
    refresh_search_projection(&transaction, "company", &company.id)?;
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
    refresh_search_projection(&transaction, "company", &company.id)?;
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
    refresh_search_projection(&transaction, "company", &company.id)?;
    refresh_activity_projections_for_parent(&transaction, "company", &company.id)?;
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
    refresh_search_projection(&transaction, "contact", &contact_id)?;
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
    refresh_search_projection(&transaction, "contact", &existing.id)?;
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
    refresh_search_projection(&transaction, "contact", &contact.id)?;
    refresh_activity_projections_for_parent(&transaction, "contact", &contact.id)?;
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
    refresh_search_projection(&transaction, "opportunity", &opportunity_id)?;
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
    refresh_search_projection(&transaction, "opportunity", &existing.id)?;
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
    refresh_search_projection(&transaction, "opportunity", &opportunity.id)?;
    refresh_activity_projections_for_parent(&transaction, "opportunity", &opportunity.id)?;
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
    refresh_search_projection(&transaction, "opportunity", &existing.id)?;
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
    refresh_search_projection(&transaction, "activity", &activity.id)?;
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
    refresh_search_projection(&transaction, "activity", &activity.id)?;
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
    refresh_search_projection(&transaction, "activity", &activity.id)?;
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
        refresh_search_projection(&transaction, "activity", &activity_id)?;
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

/// Gather the facts the attention rules run on (last activity per contact
/// including related opportunities, stage entry times, open tasks).
/// `reference_time` defaults to now. Exposed so the AI explanation layer can
/// quote the very facts a flag was computed from instead of re-deriving them.
pub fn attention_inputs(
    storage: &Storage,
    reference_time: Option<String>,
) -> Result<AttentionInputs, ApplicationError> {
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
    Ok(AttentionInputs {
        reference_time,
        thresholds: get_attention_thresholds(storage)?,
        contacts: load_contact_facts(connection)?,
        opportunities: load_opportunity_facts(connection)?,
        tasks: load_task_facts(connection)?,
    })
}

/// Compute the needs-attention flags from those facts with the pure rules in
/// `attention`. Results are never stored.
pub fn get_attention_flags(
    storage: &Storage,
    reference_time: Option<String>,
) -> Result<Vec<AttentionFlag>, ApplicationError> {
    Ok(attention::evaluate(&attention_inputs(
        storage,
        reference_time,
    )?))
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

fn read_navigation_recents(
    connection: &rusqlite::Connection,
) -> Result<Vec<RecentRecord>, ApplicationError> {
    let Some(value) = read_setting(connection, NAVIGATION_RECENTS_KEY)? else {
        return Ok(Vec::new());
    };
    let entries = serde_json::from_str::<Vec<RecentRecord>>(&value).map_err(|error| {
        ApplicationError::InvalidStoredData(format!(
            "app_settings {NAVIGATION_RECENTS_KEY} holds invalid JSON: {error}"
        ))
    })?;
    if entries.len() > MAX_NAVIGATION_RECENTS {
        return Err(ApplicationError::InvalidStoredData(format!(
            "app_settings {NAVIGATION_RECENTS_KEY} has more than {MAX_NAVIGATION_RECENTS} entries"
        )));
    }
    for entry in &entries {
        validate_recent_record(entry)?;
    }
    Ok(entries)
}

fn validate_recent_record(entry: &RecentRecord) -> Result<(), ApplicationError> {
    if !matches!(
        entry.entity_type.as_str(),
        "contact" | "company" | "opportunity"
    ) {
        return Err(ApplicationError::InvalidStoredData(format!(
            "app_settings {NAVIGATION_RECENTS_KEY} contains unsupported entity type {:?}",
            entry.entity_type
        )));
    }
    if entry.entity_id.trim().is_empty() {
        return Err(ApplicationError::InvalidStoredData(format!(
            "app_settings {NAVIGATION_RECENTS_KEY} contains an empty entity id"
        )));
    }
    Ok(())
}

fn navigation_resource(entity_type: &str) -> &'static str {
    match entity_type {
        "contact" => "contact",
        "company" => "company",
        "opportunity" => "opportunity",
        _ => "record",
    }
}

fn active_navigation_result(
    connection: &rusqlite::Connection,
    entry: &RecentRecord,
) -> Result<Option<SearchResult>, ApplicationError> {
    let title = match entry.entity_type.as_str() {
        "contact" => connection
            .query_row(
                "SELECT display_name FROM contacts WHERE id = ?1 AND archived_at IS NULL",
                [&entry.entity_id],
                |row| row.get(0),
            )
            .optional()?,
        "company" => connection
            .query_row(
                "SELECT name FROM companies WHERE id = ?1 AND archived_at IS NULL",
                [&entry.entity_id],
                |row| row.get(0),
            )
            .optional()?,
        "opportunity" => connection
            .query_row(
                "SELECT name FROM opportunities WHERE id = ?1 AND archived_at IS NULL",
                [&entry.entity_id],
                |row| row.get(0),
            )
            .optional()?,
        _ => return Ok(None),
    };
    Ok(title.map(|title| SearchResult {
        entity_type: entry.entity_type.clone(),
        entity_id: entry.entity_id.clone(),
        title,
        parent_type: None,
        parent_id: None,
    }))
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

// Draft checks — the proposal engine runs exactly the rules the direct
// create/update commands run, without writing anything.

/// Validate a drafted contact patch; the result is discarded.
pub fn check_contact_patch(patch: &ContactPatch) -> Result<(), ApplicationError> {
    validate_contact_patch(patch.clone()).map(|_| ())
}

/// Validate a drafted company patch; the result is discarded.
pub fn check_company_patch(patch: &CompanyPatch) -> Result<(), ApplicationError> {
    validate_company_patch(patch.clone()).map(|_| ())
}

/// Validate a drafted opportunity patch; the result is discarded.
pub fn check_opportunity_patch(patch: &OpportunityPatch) -> Result<(), ApplicationError> {
    validate_opportunity_patch(patch.clone()).map(|_| ())
}

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

fn validate_saved_view_name(value: String) -> Result<String, ApplicationError> {
    required_text("name", value, 120)
}

fn validate_saved_view_definition(
    entity_type: SavedViewEntityType,
    definition: SavedViewDefinition,
) -> Result<SavedViewDefinition, ApplicationError> {
    if definition.schema_version != SAVED_VIEW_SCHEMA_VERSION {
        return Err(ApplicationError::ValidationFailed {
            code: "unsupported_saved_view_schema_version",
            field: "definition.schemaVersion".into(),
            message: format!("must be saved-view schema version {SAVED_VIEW_SCHEMA_VERSION}"),
        });
    }
    if definition.filter.tag_ids_all.len() > 20 || definition.filter.custom_fields.len() > 10 {
        return Err(ApplicationError::ValidationFailed {
            code: "saved_view_filter_limit",
            field: "definition.filter".into(),
            message: "contains too many predicates".into(),
        });
    }
    if definition
        .filter
        .tag_ids_all
        .iter()
        .any(|id| id.trim().is_empty())
        || definition
            .filter
            .tag_ids_all
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            != definition.filter.tag_ids_all.len()
    {
        return Err(ApplicationError::ValidationFailed {
            code: "invalid_saved_view_tags",
            field: "definition.filter.tagIdsAll".into(),
            message: "must contain unique non-empty ids".into(),
        });
    }
    let mut custom_field_ids = std::collections::HashSet::new();
    for (index, predicate) in definition.filter.custom_fields.iter().enumerate() {
        if !custom_field_ids.insert(&predicate.definition_id) {
            return Err(ApplicationError::ValidationFailed {
                code: "invalid_saved_view_custom_field",
                field: format!("definition.filter.customFields[{index}].definitionId"),
                message: "must reference each definition at most once".into(),
            });
        }
        validate_predicate(index, predicate)?;
    }
    let allowed = match entity_type {
        SavedViewEntityType::Contact => ["displayName"].as_slice(),
        SavedViewEntityType::Company => ["name"].as_slice(),
        SavedViewEntityType::Opportunity => ["name", "stage", "value", "expectedClose"].as_slice(),
    };
    if !allowed.contains(&definition.sort.field.as_str()) {
        return Err(ApplicationError::ValidationFailed {
            code: "invalid_saved_view_sort",
            field: "definition.sort.field".into(),
            message: format!(
                "is not supported for {} saved views",
                entity_type.as_database_value()
            ),
        });
    }
    Ok(definition)
}

type SavedViewRow = (String, String, String, String, i64, String, String, i64);

fn saved_view_raw_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SavedViewRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
    ))
}

fn finish_saved_view(row: SavedViewRow) -> Result<SavedView, ApplicationError> {
    let (id, name, entity_type_text, definition_json, sort_key, created_at, updated_at, version) =
        row;
    let entity_type = SavedViewEntityType::parse(&entity_type_text)?;
    let definition = parse_saved_view_definition(&entity_type, &definition_json)?;
    Ok(SavedView {
        id,
        name,
        entity_type,
        definition,
        sort_key,
        created_at,
        updated_at,
        version,
    })
}

/// Decode v1 or the one known pre-versioned legacy shape. This intentionally
/// does not write to SQLite: unreadable future records stay recoverable and
/// callers can choose a deliberate repair instead of losing data on read.
fn parse_saved_view_definition(
    entity_type: &SavedViewEntityType,
    definition_json: &str,
) -> Result<SavedViewDefinition, ApplicationError> {
    let value: serde_json::Value = serde_json::from_str(definition_json).map_err(|error| {
        ApplicationError::InvalidStoredData(format!("saved view definition is not JSON: {error}"))
    })?;
    let definition = match value
        .get("schemaVersion")
        .and_then(serde_json::Value::as_i64)
    {
        Some(version) if version > SAVED_VIEW_SCHEMA_VERSION => {
            return Err(ApplicationError::InvalidStoredData(format!(
                "saved view definition schema version {version} is newer than supported version {SAVED_VIEW_SCHEMA_VERSION}"
            )));
        }
        Some(SAVED_VIEW_SCHEMA_VERSION) => serde_json::from_value(value).map_err(|error| {
            ApplicationError::InvalidStoredData(format!("invalid saved view definition: {error}"))
        })?,
        Some(1) => migrate_saved_view_v1(value)?,
        Some(version) if version < 0 => {
            return Err(ApplicationError::InvalidStoredData(format!(
                "saved view definition has invalid schema version {version}"
            )));
        }
        Some(version) => {
            return Err(ApplicationError::InvalidStoredData(format!(
                "saved view definition schema version {version} is not migratable"
            )));
        }
        None => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct LegacyFilterV0 {
                include_archived: bool,
            }
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct LegacyDefinitionV0 {
                filter: LegacyFilterV0,
                sort: SavedViewSort,
            }
            let legacy: LegacyDefinitionV0 = serde_json::from_value(value).map_err(|error| {
                ApplicationError::InvalidStoredData(format!(
                    "invalid saved view definition: {error}"
                ))
            })?;
            SavedViewDefinition {
                schema_version: SAVED_VIEW_SCHEMA_VERSION,
                filter: SavedViewFilter {
                    include_archived: legacy.filter.include_archived,
                    tag_ids_all: vec![],
                    custom_fields: vec![],
                },
                sort: legacy.sort,
            }
        }
    };
    validate_saved_view_definition(entity_type.clone(), definition).map_err(|error| {
        ApplicationError::InvalidStoredData(format!("invalid saved view definition: {error}"))
    })
}

fn migrate_saved_view_v1(
    value: serde_json::Value,
) -> Result<SavedViewDefinition, ApplicationError> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct V1 {
        #[serde(rename = "schemaVersion")]
        _schema_version: i64,
        filter: LegacyFilter,
        sort: SavedViewSort,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct LegacyFilter {
        include_archived: bool,
    }
    let legacy: V1 = serde_json::from_value(value).map_err(|e| {
        ApplicationError::InvalidStoredData(format!("invalid saved view definition: {e}"))
    })?;
    Ok(SavedViewDefinition {
        schema_version: SAVED_VIEW_SCHEMA_VERSION,
        filter: SavedViewFilter {
            include_archived: legacy.filter.include_archived,
            tag_ids_all: vec![],
            custom_fields: vec![],
        },
        sort: legacy.sort,
    })
}

fn tag_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Tag> {
    Ok(Tag {
        id: row.get(0)?,
        label: row.get(1)?,
        color_role: row.get(2)?,
        archived_at: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
        version: row.get(6)?,
    })
}
fn option_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CustomFieldOption> {
    Ok(CustomFieldOption {
        id: row.get(0)?,
        definition_id: row.get(1)?,
        label: row.get(2)?,
        sort_key: row.get(3)?,
    })
}
type DefRow = (
    String,
    String,
    String,
    String,
    i64,
    Option<String>,
    String,
    String,
    i64,
);
fn custom_field_def_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DefRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
    ))
}
fn finish_custom_field_def(
    connection: &rusqlite::Connection,
    row: DefRow,
) -> Result<CustomFieldDef, ApplicationError> {
    let (
        id,
        entity_type,
        label,
        field_type,
        sort_key,
        archived_at,
        created_at,
        updated_at,
        version,
    ) = row;
    let mut s=connection.prepare("SELECT id,definition_id,label,sort_key FROM custom_field_options WHERE definition_id=?1 ORDER BY sort_key,id")?;
    let options = s
        .query_map([&id], option_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(CustomFieldDef {
        id,
        entity_type: SavedViewEntityType::parse(&entity_type)?,
        label,
        field_type,
        sort_key,
        archived_at,
        created_at,
        updated_at,
        version,
        options,
    })
}
fn require_tag(connection: &rusqlite::Connection, id: &str) -> Result<Tag, ApplicationError> {
    connection.query_row("SELECT id,label,color_role,archived_at,created_at,updated_at,version FROM tags WHERE id=?1",[id],tag_from_row).optional()?.ok_or_else(||ApplicationError::NotFound{resource:"tag",id:id.into()})
}
fn require_tag_label_available(
    connection: &rusqlite::Connection,
    label: &str,
    excluded_id: Option<&str>,
) -> Result<(), ApplicationError> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM tags WHERE label=?1 COLLATE NOCASE AND (?2 IS NULL OR id != ?2))",
        params![label, excluded_id],
        |row| row.get(0),
    )?;
    if exists {
        return Err(ApplicationError::ValidationFailed {
            code: "tag_label_taken",
            field: "label".into(),
            message: "a tag with this label already exists".into(),
        });
    }
    Ok(())
}
fn require_custom_field_label_available(
    connection: &rusqlite::Connection,
    entity_type: &SavedViewEntityType,
    label: &str,
    excluded_id: Option<&str>,
) -> Result<(), ApplicationError> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM custom_field_defs WHERE entity_type=?1 AND label=?2 COLLATE NOCASE AND (?3 IS NULL OR id != ?3))",
        params![entity_type.as_database_value(), label, excluded_id],
        |row| row.get(0),
    )?;
    if exists {
        return Err(ApplicationError::ValidationFailed {
            code: "custom_field_label_taken",
            field: "label".into(),
            message: "a custom field with this label already exists for this record type".into(),
        });
    }
    Ok(())
}
fn require_custom_field_def(
    connection: &rusqlite::Connection,
    id: &str,
) -> Result<CustomFieldDef, ApplicationError> {
    let row=connection.query_row("SELECT id,entity_type,label,field_type,sort_key,archived_at,created_at,updated_at,version FROM custom_field_defs WHERE id=?1",[id],custom_field_def_from_row).optional()?.ok_or_else(||ApplicationError::NotFound{resource:"custom_field_def",id:id.into()})?;
    finish_custom_field_def(connection, row)
}
fn validate_color_role(value: Option<&str>) -> Result<(), ApplicationError> {
    if matches!(value,Some(v) if !matches!(v,"neutral"|"accent"|"attention")) {
        return Err(ApplicationError::InvalidInput {
            field: "colorRole".into(),
            message: "must be neutral, accent, or attention".into(),
        });
    }
    Ok(())
}
fn validate_field_type(value: &str) -> Result<(), ApplicationError> {
    if !matches!(value, "text" | "number" | "date" | "select") {
        return Err(ApplicationError::InvalidInput {
            field: "fieldType".into(),
            message: "must be text, number, date, or select".into(),
        });
    }
    Ok(())
}
fn limit_error(code: &'static str, field: &str, max: i64) -> ApplicationError {
    ApplicationError::ValidationFailed {
        code,
        field: field.into(),
        message: format!("may contain at most {max} entries"),
    }
}
fn validate_option_inputs(
    field_type: &str,
    options: &[CustomFieldOptionInput],
) -> Result<(), ApplicationError> {
    if field_type == "select" {
        if options.is_empty() {
            return Err(ApplicationError::InvalidInput {
                field: "options".into(),
                message: "is required for select fields".into(),
            });
        }
        if options.len() as i64 > MAX_FIELD_OPTIONS {
            return Err(limit_error(
                "custom_field_option_limit_reached",
                "options",
                MAX_FIELD_OPTIONS,
            ));
        }
    } else if !options.is_empty() {
        return Err(ApplicationError::InvalidInput {
            field: "options".into(),
            message: "are only supported for select fields".into(),
        });
    }
    let mut labels = std::collections::HashSet::new();
    let mut ids = std::collections::HashSet::new();
    for option in options {
        let label = required_text("options.label", option.label.clone(), 120)?;
        if !labels.insert(label.to_lowercase()) {
            return Err(ApplicationError::ValidationFailed {
                code: "duplicate_custom_field_option",
                field: "options".into(),
                message: "labels must be unique".into(),
            });
        }
        if option.id.as_ref().is_some_and(|id| !ids.insert(id)) {
            return Err(ApplicationError::ValidationFailed {
                code: "duplicate_custom_field_option",
                field: "options.id".into(),
                message: "option ids must be unique".into(),
            });
        }
    }
    Ok(())
}
fn replace_options(
    transaction: &Transaction<'_>,
    definition_id: &str,
    options: &[CustomFieldOptionInput],
) -> Result<(), ApplicationError> {
    let old = transaction
        .prepare("SELECT id FROM custom_field_options WHERE definition_id=?1")?
        .query_map([definition_id], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let keep = options
        .iter()
        .filter_map(|o| o.id.as_deref())
        .collect::<std::collections::HashSet<_>>();
    for id in old {
        if !keep.contains(id.as_str()) {
            let used: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM custom_field_values WHERE option_id=?1)",
                [&id],
                |r| r.get(0),
            )?;
            if used {
                return Err(ApplicationError::ValidationFailed {
                    code: "custom_field_option_in_use",
                    field: "options".into(),
                    message: "referenced options cannot be removed".into(),
                });
            }
            transaction.execute("DELETE FROM custom_field_options WHERE id=?1", [id])?;
        }
    }
    for (sort_key, option) in options.iter().enumerate() {
        let label = required_text("options.label", option.label.clone(), 120)?;
        match &option.id {
            Some(id) => {
                let changed=transaction.execute("UPDATE custom_field_options SET label=?2,sort_key=?3 WHERE id=?1 AND definition_id=?4",params![id,label,sort_key as i64,definition_id])?;
                if changed == 0 {
                    return Err(ApplicationError::ValidationFailed {
                        code: "invalid_custom_field_option",
                        field: "options.id".into(),
                        message: "does not belong to this definition".into(),
                    });
                }
            }
            None => {
                transaction.execute("INSERT INTO custom_field_options (id,definition_id,label,sort_key) VALUES (?1,?2,?3,?4)",params![new_id(),definition_id,label,sort_key as i64])?;
            }
        }
    }
    Ok(())
}
fn set_tag_archive(
    storage: &mut Storage,
    request: TagArchiveRequest,
    archive: bool,
) -> Result<Tag, ApplicationError> {
    let transaction = immediate(storage)?;
    let tag = require_tag(&transaction, &request.tag_id)?;
    check_version("tag", &tag.id, request.expected_version, tag.version)?;
    if archive == tag.archived_at.is_some() {
        return Err(ApplicationError::ValidationFailed {
            code: "invalid_tag_state_transition",
            field: "tagId".into(),
            message: if archive {
                "tag is already archived"
            } else {
                "tag is already active"
            }
            .into(),
        });
    }
    transaction.execute(
        "UPDATE tags SET archived_at=?2,updated_at=?3,version=?4 WHERE id=?1",
        params![
            tag.id,
            if archive { Some(now_utc()) } else { None },
            now_utc(),
            tag.version + 1
        ],
    )?;
    log_command(
        &transaction,
        request.actor,
        "tag",
        &tag.id,
        if archive {
            "archived tag"
        } else {
            "unarchived tag"
        },
    )?;
    let tag = require_tag(&transaction, &tag.id)?;
    transaction.commit()?;
    Ok(tag)
}
fn set_def_archive(
    storage: &mut Storage,
    request: CustomFieldDefArchiveRequest,
    archive: bool,
) -> Result<CustomFieldDef, ApplicationError> {
    let transaction = immediate(storage)?;
    let def = require_custom_field_def(&transaction, &request.definition_id)?;
    check_version(
        "custom_field_def",
        &def.id,
        request.expected_version,
        def.version,
    )?;
    if archive == def.archived_at.is_some() {
        return Err(ApplicationError::ValidationFailed {
            code: "invalid_custom_field_state_transition",
            field: "definitionId".into(),
            message: if archive {
                "custom field is already archived"
            } else {
                "custom field is already active"
            }
            .into(),
        });
    }
    transaction.execute(
        "UPDATE custom_field_defs SET archived_at=?2,updated_at=?3,version=?4 WHERE id=?1",
        params![
            def.id,
            if archive { Some(now_utc()) } else { None },
            now_utc(),
            def.version + 1
        ],
    )?;
    log_command(
        &transaction,
        request.actor,
        "custom_field_def",
        &def.id,
        if archive {
            "archived custom field"
        } else {
            "unarchived custom field"
        },
    )?;
    let def = require_custom_field_def(&transaction, &def.id)?;
    transaction.commit()?;
    Ok(def)
}
fn owner_table(entity_type: &SavedViewEntityType) -> &'static str {
    match entity_type {
        SavedViewEntityType::Contact => "contacts",
        SavedViewEntityType::Company => "companies",
        SavedViewEntityType::Opportunity => "opportunities",
    }
}
fn owner_version(
    connection: &rusqlite::Connection,
    entity_type: &SavedViewEntityType,
    id: &str,
) -> Result<i64, ApplicationError> {
    connection
        .query_row(
            &format!(
                "SELECT version FROM {} WHERE id=?1",
                owner_table(entity_type)
            ),
            [id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| ApplicationError::NotFound {
            resource: entity_type.as_database_value(),
            id: id.into(),
        })
}
fn ensure_owner(
    connection: &rusqlite::Connection,
    entity_type: &SavedViewEntityType,
    id: &str,
) -> Result<(), ApplicationError> {
    owner_version(connection, entity_type, id).map(|_| ())
}
fn bump_owner(
    transaction: &Transaction<'_>,
    entity_type: &SavedViewEntityType,
    id: &str,
    version: i64,
    now: &str,
) -> Result<(), ApplicationError> {
    transaction.execute(
        &format!(
            "UPDATE {} SET updated_at=?2,version=?3 WHERE id=?1",
            owner_table(entity_type)
        ),
        params![id, now, version],
    )?;
    Ok(())
}
fn value_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CustomFieldValue> {
    Ok(CustomFieldValue {
        id: row.get(0)?,
        definition_id: row.get(1)?,
        entity_type: SavedViewEntityType::parse(&row.get::<_, String>(2)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        record_id: row.get(3)?,
        text_value: row.get(4)?,
        number_value: row.get(5)?,
        date_value: row.get(6)?,
        option_id: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}
fn load_metadata(
    connection: &rusqlite::Connection,
    entity_type: &SavedViewEntityType,
    record_id: &str,
) -> Result<RecordMetadata, ApplicationError> {
    let tag_ids = connection
        .prepare(
            "SELECT tag_id FROM record_tags WHERE entity_type=?1 AND record_id=?2 ORDER BY tag_id",
        )?
        .query_map(params![entity_type.as_database_value(), record_id], |r| {
            r.get(0)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let values=connection.prepare("SELECT id,definition_id,entity_type,record_id,text_value,number_value,date_value,option_id,created_at,updated_at FROM custom_field_values WHERE entity_type=?1 AND record_id=?2 ORDER BY definition_id,id")?.query_map(params![entity_type.as_database_value(),record_id],value_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(RecordMetadata { tag_ids, values })
}
fn validate_metadata_request(request: &SetRecordMetadataRequest) -> Result<(), ApplicationError> {
    if request.tag_ids.len() > 20 {
        return Err(limit_error("record_tag_limit_reached", "tagIds", 20));
    }
    if request.values.len() > 50 {
        return Err(limit_error(
            "record_custom_field_limit_reached",
            "values",
            50,
        ));
    }
    let mut tags = std::collections::HashSet::new();
    for tag in &request.tag_ids {
        if tag.trim().is_empty() || !tags.insert(tag) {
            return Err(ApplicationError::InvalidInput {
                field: "tagIds".into(),
                message: "must contain unique ids".into(),
            });
        }
    }
    let mut definitions = std::collections::HashSet::new();
    for value in &request.values {
        if !definitions.insert(&value.definition_id) {
            return Err(ApplicationError::InvalidInput {
                field: "values".into(),
                message: "must contain one value per definition".into(),
            });
        }
        validate_value_shape(value)?
    }
    Ok(())
}
fn validate_value_shape(value: &CustomFieldValueInput) -> Result<(), ApplicationError> {
    let count = [
        value.text_value.is_some(),
        value.number_value.is_some(),
        value.date_value.is_some(),
        value.option_id.is_some(),
    ]
    .into_iter()
    .filter(|x| *x)
    .count();
    if count != 1 {
        return Err(ApplicationError::InvalidInput {
            field: "values".into(),
            message: "each value needs exactly one typed value".into(),
        });
    }
    if let Some(text) = &value.text_value {
        if text.chars().count() > 4000 {
            return Err(ApplicationError::InvalidInput {
                field: "values.textValue".into(),
                message: "must be 4,000 characters or fewer".into(),
            });
        }
    }
    if let Some(number) = value.number_value {
        if !number.is_finite() || number.abs() > 1_000_000_000_000_000.0 {
            return Err(ApplicationError::InvalidInput {
                field: "values.numberValue".into(),
                message: "must be finite and within range".into(),
            });
        }
    }
    if let Some(date) = &value.date_value {
        NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|_| {
            ApplicationError::InvalidInput {
                field: "values.dateValue".into(),
                message: "must be YYYY-MM-DD".into(),
            }
        })?;
    }
    Ok(())
}
fn validate_metadata_references(
    connection: &rusqlite::Connection,
    request: &SetRecordMetadataRequest,
    existing: &RecordMetadata,
) -> Result<(), ApplicationError> {
    for tag in &request.tag_ids {
        let stored = require_tag(connection, tag)?;
        if stored.archived_at.is_some() && !existing.tag_ids.contains(tag) {
            return Err(ApplicationError::ValidationFailed {
                code: "invalid_tag",
                field: "tagIds".into(),
                message: "references an unavailable tag".into(),
            });
        }
    }
    for value in &request.values {
        let def = require_custom_field_def(connection, &value.definition_id)?;
        if def.entity_type != request.entity_type {
            return Err(ApplicationError::ValidationFailed {
                code: "invalid_custom_field_definition",
                field: "values.definitionId".into(),
                message: "does not apply to this record".into(),
            });
        }
        if def.archived_at.is_some()
            && !existing.values.iter().any(|stored| {
                stored.definition_id == value.definition_id
                    && stored.text_value == value.text_value
                    && stored.number_value == value.number_value
                    && stored.date_value == value.date_value
                    && stored.option_id == value.option_id
            })
        {
            return Err(ApplicationError::ValidationFailed {
                code: "invalid_custom_field_definition",
                field: "values.definitionId".into(),
                message: "archived custom fields may only retain their existing value".into(),
            });
        }
        match (def.field_type.as_str(), &value.option_id) {
            ("text", None) if value.text_value.is_some() => (),
            ("number", None) if value.number_value.is_some() => (),
            ("date", None) if value.date_value.is_some() => (),
            ("select", Some(option)) => {
                if !def.options.iter().any(|o| &o.id == option) {
                    return Err(ApplicationError::ValidationFailed {
                        code: "invalid_custom_field_option",
                        field: "values.optionId".into(),
                        message: "does not belong to the definition".into(),
                    });
                }
            }
            _ => {
                return Err(ApplicationError::ValidationFailed {
                    code: "custom_field_type_mismatch",
                    field: "values".into(),
                    message: "does not match its definition type".into(),
                })
            }
        }
    }
    Ok(())
}
fn metadata_equal(existing: &RecordMetadata, request: &SetRecordMetadataRequest) -> bool {
    if existing.tag_ids.len() != request.tag_ids.len()
        || !existing
            .tag_ids
            .iter()
            .all(|id| request.tag_ids.contains(id))
    {
        return false;
    }
    if existing.values.len() != request.values.len() {
        return false;
    }
    existing.values.iter().all(|existing_value| {
        request
            .values
            .iter()
            .find(|candidate| candidate.definition_id == existing_value.definition_id)
            .is_some_and(|candidate| {
                existing_value.text_value == candidate.text_value
                    && existing_value.number_value == candidate.number_value
                    && existing_value.date_value == candidate.date_value
                    && existing_value.option_id == candidate.option_id
            })
    })
}
fn validate_predicate(
    index: usize,
    p: &SavedViewCustomFieldPredicate,
) -> Result<(), ApplicationError> {
    let valid = match p.field_type.as_str() {
        "text" => {
            matches!(p.operator.as_str(), "contains" | "equals")
                && p.value
                    .as_str()
                    .is_some_and(|value| !value.is_empty() && value.chars().count() <= 4_000)
        }
        "number" => {
            matches!(
                p.operator.as_str(),
                "equals" | "greaterThanOrEqual" | "lessThanOrEqual"
            ) && p
                .value
                .as_f64()
                .is_some_and(|v| v.is_finite() && v.abs() <= 1_000_000_000_000_000.0)
        }
        "date" => {
            matches!(p.operator.as_str(), "on" | "before" | "after")
                && p.value
                    .as_str()
                    .is_some_and(|v| NaiveDate::parse_from_str(v, "%Y-%m-%d").is_ok())
        }
        "select" => p.operator == "is" && p.value.as_str().is_some_and(|v| !v.is_empty()),
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(ApplicationError::ValidationFailed {
            code: "invalid_saved_view_custom_field",
            field: format!("definition.filter.customFields[{index}]"),
            message: "has an unsupported type, operator, or value".into(),
        })
    }
}
fn validate_saved_view_references(
    connection: &rusqlite::Connection,
    entity_type: &SavedViewEntityType,
    definition: &SavedViewDefinition,
) -> Result<(), ApplicationError> {
    for tag in &definition.filter.tag_ids_all {
        let exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM tags WHERE id=?1 AND archived_at IS NULL)",
            [tag],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(ApplicationError::ValidationFailed {
                code: "stale_saved_view_reference",
                field: "definition.filter.tagIdsAll".into(),
                message: "references a missing tag".into(),
            });
        }
    }
    for p in &definition.filter.custom_fields {
        let def = match require_custom_field_def(connection, &p.definition_id) {
            Ok(definition) => definition,
            Err(ApplicationError::NotFound { .. }) => {
                return Err(ApplicationError::ValidationFailed {
                    code: "stale_saved_view_reference",
                    field: "definition.filter.customFields".into(),
                    message: "references a missing custom field".into(),
                });
            }
            Err(error) => return Err(error),
        };
        if &def.entity_type != entity_type
            || def.field_type != p.field_type
            || def.archived_at.is_some()
        {
            return Err(ApplicationError::ValidationFailed {
                code: "stale_saved_view_reference",
                field: "definition.filter.customFields".into(),
                message: "references an incompatible custom field".into(),
            });
        }
        if let Some(option) = p.value.as_str().filter(|_| p.field_type == "select") {
            if !def.options.iter().any(|o| o.id == option) {
                return Err(ApplicationError::ValidationFailed {
                    code: "stale_saved_view_reference",
                    field: "definition.filter.customFields".into(),
                    message: "references a missing option".into(),
                });
            }
        }
    }
    Ok(())
}
fn matches_predicate(
    connection: &rusqlite::Connection,
    entity_type: &str,
    record_id: &str,
    p: &SavedViewCustomFieldPredicate,
) -> Result<bool, ApplicationError> {
    let (sql,value):(&str,rusqlite::types::Value)=match (p.field_type.as_str(),p.operator.as_str()){("text","contains")=>("SELECT EXISTS(SELECT 1 FROM custom_field_values WHERE definition_id=?1 AND entity_type=?2 AND record_id=?3 AND instr(text_value,?4)>0)",p.value.as_str().unwrap().to_owned().into()),("text","equals")=>("SELECT EXISTS(SELECT 1 FROM custom_field_values WHERE definition_id=?1 AND entity_type=?2 AND record_id=?3 AND text_value=?4)",p.value.as_str().unwrap().to_owned().into()),("number","equals")=>("SELECT EXISTS(SELECT 1 FROM custom_field_values WHERE definition_id=?1 AND entity_type=?2 AND record_id=?3 AND number_value=?4)",p.value.as_f64().unwrap().into()),("number","greaterThanOrEqual")=>("SELECT EXISTS(SELECT 1 FROM custom_field_values WHERE definition_id=?1 AND entity_type=?2 AND record_id=?3 AND number_value>=?4)",p.value.as_f64().unwrap().into()),("number","lessThanOrEqual")=>("SELECT EXISTS(SELECT 1 FROM custom_field_values WHERE definition_id=?1 AND entity_type=?2 AND record_id=?3 AND number_value<=?4)",p.value.as_f64().unwrap().into()),("date","on")=>("SELECT EXISTS(SELECT 1 FROM custom_field_values WHERE definition_id=?1 AND entity_type=?2 AND record_id=?3 AND date_value=?4)",p.value.as_str().unwrap().to_owned().into()),("date","before")=>("SELECT EXISTS(SELECT 1 FROM custom_field_values WHERE definition_id=?1 AND entity_type=?2 AND record_id=?3 AND date_value<?4)",p.value.as_str().unwrap().to_owned().into()),("date","after")=>("SELECT EXISTS(SELECT 1 FROM custom_field_values WHERE definition_id=?1 AND entity_type=?2 AND record_id=?3 AND date_value>?4)",p.value.as_str().unwrap().to_owned().into()),("select","is")=>("SELECT EXISTS(SELECT 1 FROM custom_field_values WHERE definition_id=?1 AND entity_type=?2 AND record_id=?3 AND option_id=?4)",p.value.as_str().unwrap().to_owned().into()),_=>return Ok(false)};
    connection
        .query_row(
            sql,
            params![p.definition_id, entity_type, record_id, value],
            |r| r.get(0),
        )
        .map_err(Into::into)
}

fn require_saved_view_name_available(
    connection: &rusqlite::Connection,
    entity_type: &str,
    name: &str,
    excluded_id: Option<&str>,
) -> Result<(), ApplicationError> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM saved_views
         WHERE entity_type = ?1 AND name = ?2 COLLATE NOCASE
           AND (?3 IS NULL OR id != ?3))",
        params![entity_type, name, excluded_id],
        |row| row.get(0),
    )?;
    if exists {
        return Err(ApplicationError::ValidationFailed {
            code: "saved_view_name_taken",
            field: "name".into(),
            message: "a saved view with this name already exists for this list".into(),
        });
    }
    Ok(())
}

fn require_saved_view(
    connection: &rusqlite::Connection,
    saved_view_id: &str,
) -> Result<SavedView, ApplicationError> {
    let row = connection
        .query_row(
            "SELECT id, name, entity_type, definition_json, sort_key, created_at, updated_at, version
             FROM saved_views WHERE id = ?1",
            [saved_view_id],
            saved_view_raw_from_row,
        )
        .optional()?
        .ok_or_else(|| ApplicationError::NotFound {
            resource: "saved_view",
            id: saved_view_id.into(),
        })?;
    finish_saved_view(row)
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

pub(crate) fn immediate(storage: &mut Storage) -> Result<Transaction<'_>, ApplicationError> {
    Ok(storage
        .connection_mut()
        .transaction_with_behavior(TransactionBehavior::Immediate)?)
}

/// Replace a record's FTS projection from the canonical rows. This is called
/// only from the transaction that changed the record, so a failed write cannot
/// leave the index ahead of or behind its source data.
fn refresh_search_projection(
    transaction: &Transaction<'_>,
    entity_type: &str,
    entity_id: &str,
) -> Result<(), ApplicationError> {
    transaction.execute(
        "DELETE FROM search_index WHERE entity_type = ?1 AND entity_id = ?2",
        params![entity_type, entity_id],
    )?;
    let inserted = match entity_type {
        "company" => transaction.execute(
            "INSERT INTO search_index (entity_type, entity_id, title, content)
             SELECT 'company', id, name, trim(coalesce(name, '') || ' ' || coalesce(phone, '') || ' ' || coalesce(email, '') || ' ' || coalesce(website, '') || ' ' || coalesce(notes, ''))
             FROM companies WHERE id = ?1 AND archived_at IS NULL",
            [entity_id],
        )?,
        "contact" => transaction.execute(
            "INSERT INTO search_index (entity_type, entity_id, title, content)
             SELECT 'contact', c.id, c.display_name, trim(coalesce(c.display_name, '') || ' ' || coalesce(c.notes, '') || ' ' || coalesce((SELECT group_concat(value, ' ') FROM contact_channels cc WHERE cc.contact_id = c.id), ''))
             FROM contacts c WHERE c.id = ?1 AND c.archived_at IS NULL",
            [entity_id],
        )?,
        "opportunity" => transaction.execute(
            "INSERT INTO search_index (entity_type, entity_id, title, content)
             SELECT 'opportunity', id, name, trim(coalesce(name, '') || ' ' || coalesce(notes, '') || ' ' || coalesce(source_label, ''))
             FROM opportunities WHERE id = ?1 AND archived_at IS NULL",
            [entity_id],
        )?,
        "activity" => transaction.execute(
            "INSERT INTO search_index (entity_type, entity_id, title, content)
             SELECT 'activity', a.id, a.summary,
                    trim(coalesce(a.summary, '') || ' ' || coalesce(a.body, ''))
             FROM activities a WHERE a.id = ?1 AND (
                 (a.parent_type = 'contact' AND EXISTS (
                     SELECT 1 FROM contacts c
                     WHERE c.id = a.parent_id AND c.archived_at IS NULL
                 )) OR
                 (a.parent_type = 'company' AND EXISTS (
                     SELECT 1 FROM companies c
                     WHERE c.id = a.parent_id AND c.archived_at IS NULL
                 )) OR
                 (a.parent_type = 'opportunity' AND EXISTS (
                     SELECT 1 FROM opportunities o
                     WHERE o.id = a.parent_id AND o.archived_at IS NULL
                 ))
             )",
            [entity_id],
        )?,
        _ => unreachable!("only indexed entity types are refreshed"),
    };
    debug_assert!(inserted <= 1);
    Ok(())
}

/// Rebuild the whole FTS projection from the canonical tables. Used by the
/// portable archive import, which replaces every canonical row at once and so
/// cannot refresh record by record. Mirrors the migration 006 backfill.
pub(crate) fn rebuild_search_index(transaction: &Transaction<'_>) -> Result<(), ApplicationError> {
    transaction.execute_batch(
        "DELETE FROM search_index;
         INSERT INTO search_index (entity_type, entity_id, title, content)
         SELECT 'company', id, name,
                trim(coalesce(name, '') || ' ' || coalesce(phone, '') || ' ' ||
                     coalesce(email, '') || ' ' || coalesce(website, '') || ' ' ||
                     coalesce(notes, ''))
         FROM companies WHERE archived_at IS NULL;
         INSERT INTO search_index (entity_type, entity_id, title, content)
         SELECT 'contact', c.id, c.display_name,
                trim(coalesce(c.display_name, '') || ' ' || coalesce(c.notes, '') || ' ' ||
                     coalesce((SELECT group_concat(value, ' ') FROM contact_channels cc
                               WHERE cc.contact_id = c.id), ''))
         FROM contacts c WHERE c.archived_at IS NULL;
         INSERT INTO search_index (entity_type, entity_id, title, content)
         SELECT 'opportunity', id, name,
                trim(coalesce(name, '') || ' ' || coalesce(notes, '') || ' ' ||
                     coalesce(source_label, ''))
         FROM opportunities WHERE archived_at IS NULL;
         INSERT INTO search_index (entity_type, entity_id, title, content)
         SELECT 'activity', a.id, a.summary,
                trim(coalesce(a.summary, '') || ' ' || coalesce(a.body, ''))
         FROM activities a
         WHERE (a.parent_type = 'contact' AND EXISTS (
                    SELECT 1 FROM contacts c WHERE c.id = a.parent_id AND c.archived_at IS NULL
                ))
            OR (a.parent_type = 'company' AND EXISTS (
                    SELECT 1 FROM companies c WHERE c.id = a.parent_id AND c.archived_at IS NULL
                ))
            OR (a.parent_type = 'opportunity' AND EXISTS (
                    SELECT 1 FROM opportunities o WHERE o.id = a.parent_id AND o.archived_at IS NULL
                ));",
    )?;
    Ok(())
}

/// Refresh every activity directly owned by a parent when its archive state
/// changes. Canonical activity rows stay intact; only the active-record search
/// projection is removed or rebuilt, inside the parent's write transaction.
fn refresh_activity_projections_for_parent(
    transaction: &Transaction<'_>,
    parent_type: &str,
    parent_id: &str,
) -> Result<(), ApplicationError> {
    let mut statement = transaction
        .prepare("SELECT id FROM activities WHERE parent_type = ?1 AND parent_id = ?2")?;
    let activity_ids = statement
        .query_map(params![parent_type, parent_id], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    for activity_id in activity_ids {
        refresh_search_projection(transaction, "activity", &activity_id)?;
    }
    Ok(())
}

/// Convert caller text into a literal, bounded AND query. Splitting prevents
/// FTS operators, prefixes, and quoting from changing query semantics.
fn bounded_fts_query(query: &str) -> String {
    query
        .chars()
        .take(128)
        .collect::<String>()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .take(12)
        .map(|word| format!("\"{word}\""))
        .collect::<Vec<_>>()
        .join(" AND ")
}

pub(crate) fn check_version(
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
pub(crate) fn log_command(
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

// ---------------------------------------------------------------------------
// CSV import/export
// ---------------------------------------------------------------------------

/// Sample rows returned by the import preview.
const IMPORT_SAMPLE_ROWS: usize = 50;
/// Contact kind used for records an import creates without a kind column.
const IMPORT_DEFAULT_KIND: &str = "client";
/// Leading characters spreadsheets treat as the start of a formula.
const FORMULA_PREFIXES: [char; 5] = ['=', '+', '-', '@', '\t'];

/// Import targets in mapping order with the header aliases the auto-guess
/// accepts. Aliases are normalized (lowercase, alphanumeric only) and the
/// first alias that matches an unclaimed header wins.
const IMPORT_TARGETS: &[(&str, &[&str])] = &[
    (
        "externalId",
        &["externalid", "externalref", "sourceid", "recordid", "id"],
    ),
    ("firstName", &["firstname", "first", "givenname"]),
    ("lastName", &["lastname", "last", "surname", "familyname"]),
    (
        "displayName",
        &["displayname", "fullname", "contactname", "name"],
    ),
    ("role", &["role", "title", "jobtitle"]),
    ("kind", &["kind", "contacttype", "partykind", "type"]),
    (
        "preferredContactMethod",
        &[
            "preferredcontactmethod",
            "preferredmethod",
            "contactmethod",
            "preferredcontact",
        ],
    ),
    (
        "addressLine1",
        &[
            "addressline1",
            "address1",
            "streetaddress",
            "street",
            "address",
        ],
    ),
    (
        "addressLine2",
        &["addressline2", "address2", "suite", "unit"],
    ),
    ("city", &["city", "town"]),
    ("state", &["state", "province", "region"]),
    ("postalCode", &["postalcode", "zipcode", "zip", "postcode"]),
    ("propertyType", &["propertytype", "property"]),
    ("notes", &["notes", "note", "comments", "description"]),
    (
        "company",
        &[
            "companyname",
            "company",
            "organization",
            "organisation",
            "account",
            "business",
        ],
    ),
    (
        "email",
        &["emailaddress", "email", "primaryemail", "email1"],
    ),
    (
        "phone",
        &[
            "phonenumber",
            "phone",
            "primaryphone",
            "mobile",
            "cell",
            "telephone",
            "tel",
        ],
    ),
    ("tags", &["tags", "tag", "labels"]),
];

/// CSV column mapping: each importable contact target names the CSV header it
/// reads from. Unset targets are simply not imported.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct ContactImportMapping {
    pub external_id: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub display_name: Option<String>,
    pub role: Option<String>,
    pub kind: Option<String>,
    pub preferred_contact_method: Option<String>,
    pub address_line1: Option<String>,
    pub address_line2: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub postal_code: Option<String>,
    pub property_type: Option<String>,
    pub notes: Option<String>,
    pub company: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub tags: Option<String>,
}

impl ContactImportMapping {
    /// Header currently mapped to a target key from `IMPORT_TARGETS`.
    fn header_for(&self, target: &str) -> Option<&str> {
        let value = match target {
            "externalId" => &self.external_id,
            "firstName" => &self.first_name,
            "lastName" => &self.last_name,
            "displayName" => &self.display_name,
            "role" => &self.role,
            "kind" => &self.kind,
            "preferredContactMethod" => &self.preferred_contact_method,
            "addressLine1" => &self.address_line1,
            "addressLine2" => &self.address_line2,
            "city" => &self.city,
            "state" => &self.state,
            "postalCode" => &self.postal_code,
            "propertyType" => &self.property_type,
            "notes" => &self.notes,
            "company" => &self.company,
            "email" => &self.email,
            "phone" => &self.phone,
            "tags" => &self.tags,
            _ => unreachable!("unknown import target"),
        };
        value.as_deref()
    }

    fn set(&mut self, target: &str, header: String) {
        let slot = match target {
            "externalId" => &mut self.external_id,
            "firstName" => &mut self.first_name,
            "lastName" => &mut self.last_name,
            "displayName" => &mut self.display_name,
            "role" => &mut self.role,
            "kind" => &mut self.kind,
            "preferredContactMethod" => &mut self.preferred_contact_method,
            "addressLine1" => &mut self.address_line1,
            "addressLine2" => &mut self.address_line2,
            "city" => &mut self.city,
            "state" => &mut self.state,
            "postalCode" => &mut self.postal_code,
            "propertyType" => &mut self.property_type,
            "notes" => &mut self.notes,
            "company" => &mut self.company,
            "email" => &mut self.email,
            "phone" => &mut self.phone,
            "tags" => &mut self.tags,
            _ => unreachable!("unknown import target"),
        };
        *slot = Some(header);
    }
}

/// One row the import refused, identified by its 1-based CSV line number.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContactImportIssue {
    pub line: u64,
    pub reason: String,
}

/// Read-only look at a CSV file before anything is written.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContactImportPreview {
    pub headers: Vec<String>,
    pub row_count: usize,
    /// Effective mapping: the caller's when given, otherwise the auto-guess.
    pub mapping: ContactImportMapping,
    /// Up to `IMPORT_SAMPLE_ROWS` data rows in file order.
    pub sample_rows: Vec<Vec<String>>,
    /// Validation problems found in the sampled rows under the mapping.
    pub issues: Vec<ContactImportIssue>,
}

/// Apply a mapped CSV file to the contact table in one transaction.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportContactsRequest {
    /// Imports log as `import` unless the caller says otherwise.
    #[serde(default = "import_actor")]
    pub actor: Actor,
    pub path: String,
    pub mapping: ContactImportMapping,
}

fn import_actor() -> Actor {
    Actor::Import
}

/// What an import did; skipped rows carry their line and reason.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContactImportSummary {
    pub created: usize,
    pub updated: usize,
    pub skipped: Vec<ContactImportIssue>,
}

/// Where a CSV export landed and how many records it holds.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CsvExportReport {
    pub path: String,
    pub row_count: usize,
}

/// A prepared import row. `patch` holds only the cells the file actually
/// carried (blank cells are None) so updates can leave everything else alone;
/// `fields` is the validated create shape with import defaults applied.
struct PreparedImportRow {
    line: u64,
    patch: ContactPatch,
    kind_mapped: bool,
    fields: ValidContactFields,
    external_id: Option<String>,
    company_name: Option<String>,
    tags: Vec<String>,
}

/// An existing contact an import row matched.
struct ImportMatch {
    id: String,
    archived: bool,
}

/// Parse a CSV file's header and rows without touching the database.
pub fn preview_contact_import(
    path: &str,
    mapping: Option<ContactImportMapping>,
) -> Result<ContactImportPreview, ApplicationError> {
    let (headers, records) = read_csv_records(path)?;
    let mapping = match mapping {
        Some(mapping) => mapping,
        None => guess_contact_mapping(&headers),
    };
    let indexes = mapping_indexes(&headers, &mapping)?;

    let mut sample_rows = Vec::new();
    let mut issues = Vec::new();
    for (line, record) in records.iter().take(IMPORT_SAMPLE_ROWS) {
        sample_rows.push(record.iter().map(str::to_owned).collect::<Vec<_>>());
        if let Err(reason) = prepare_import_row(*line, record, &indexes) {
            issues.push(reason);
        }
    }
    Ok(ContactImportPreview {
        headers,
        row_count: records.len(),
        mapping,
        sample_rows,
        issues,
    })
}

/// Apply a whole mapped CSV file: valid rows are created or updated by external
/// id, invalid rows are skipped and reported. One immediate transaction covers
/// the file, so a failure leaves the database untouched. Updates are patches —
/// a column the file does not carry (or carries blank) never clears a stored
/// value, and channels are only ever added.
pub fn import_contacts(
    storage: &mut Storage,
    request: ImportContactsRequest,
) -> Result<ContactImportSummary, ApplicationError> {
    let (headers, records) = read_csv_records(&request.path)?;
    let indexes = mapping_indexes(&headers, &request.mapping)?;

    let mut prepared = Vec::new();
    let mut skipped = Vec::new();
    for (line, record) in &records {
        match prepare_import_row(*line, record, &indexes) {
            Ok(row) => prepared.push(row),
            Err(issue) => skipped.push(issue),
        }
    }

    let actor = request.actor;
    let mut created = 0usize;
    let mut updated = 0usize;
    let transaction = immediate(storage)?;
    for row in prepared {
        let company_id = match &row.company_name {
            Some(name) => Some(resolve_import_company(&transaction, name, actor)?),
            None => None,
        };
        match find_contact_for_import(&transaction, row.external_id.as_deref())? {
            // Archived contacts stay out of the way of imports; unarchive first.
            Some(matched) if matched.archived => skipped.push(ContactImportIssue {
                line: row.line,
                reason: format!("matches archived contact {}", matched.id),
            }),
            Some(matched) => {
                update_imported_contact(&transaction, &matched.id, &row, company_id, actor)?;
                apply_import_tags(&transaction, &matched.id, &row.tags, actor)?;
                updated += 1;
            }
            None => {
                let mut fields = row.fields;
                fields.company_id = company_id;
                let contact_id = insert_imported_contact(
                    &transaction,
                    &fields,
                    row.external_id.as_deref(),
                    actor,
                )?;
                apply_import_tags(&transaction, &contact_id, &row.tags, actor)?;
                created += 1;
            }
        }
    }
    transaction.commit()?;
    skipped.sort_by_key(|issue| issue.line);
    Ok(ContactImportSummary {
        created,
        updated,
        skipped,
    })
}

/// Write every active contact to a CSV file, one column per contact custom
/// field definition. The `external_id` column falls back to the record id so an
/// exported file re-imports onto the same records.
pub fn export_contacts_csv(
    storage: &mut Storage,
    path: &str,
    overwrite: bool,
) -> Result<CsvExportReport, ApplicationError> {
    let path = check_export_destination(storage, path, overwrite)?;
    // Render before opening the destination: a failed query must never leave a
    // truncated file over a previous export.
    let mut writer = csv::Writer::from_writer(Vec::new());
    let row_count = write_contacts_csv(storage.connection(), &mut writer)?;
    write_export_file(&path, &csv_bytes(writer)?)?;
    log_export(storage, "contacts", row_count, &path)?;
    Ok(CsvExportReport { path, row_count })
}

/// Contact CSV body, shared by the file export and the portable archive's
/// human-readable copy. Returns the number of data rows written.
pub(crate) fn write_contacts_csv<W: std::io::Write>(
    connection: &rusqlite::Connection,
    writer: &mut csv::Writer<W>,
) -> Result<usize, ApplicationError> {
    let definitions = export_custom_field_defs(connection, "contact")?;
    let values = export_custom_field_values(connection, "contact")?;

    let mut headers = vec![
        "id",
        "external_id",
        "first_name",
        "last_name",
        "display_name",
        "role",
        "kind",
        "preferred_contact_method",
        "address_line1",
        "address_line2",
        "city",
        "state",
        "postal_code",
        "property_type",
        "notes",
        "favorite",
        "company",
        "email",
        "phone",
        "tags",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    headers.extend(definitions.iter().map(|(_, label)| label.clone()));
    headers.push("created_at".into());
    headers.push("updated_at".into());

    let mut statement = connection.prepare(
        "SELECT c.id, COALESCE(c.external_id, c.id), c.first_name, c.last_name, c.display_name,
                c.role, c.kind, c.preferred_contact_method, c.address_line1, c.address_line2,
                c.city, c.state, c.postal_code, c.property_type, c.notes, c.favorite,
                co.name,
                (SELECT cc.value FROM contact_channels cc
                  WHERE cc.contact_id = c.id AND cc.kind = 'email'
                  ORDER BY cc.preferred DESC, cc.sort_key, cc.id LIMIT 1),
                (SELECT cc.value FROM contact_channels cc
                  WHERE cc.contact_id = c.id AND cc.kind = 'phone'
                  ORDER BY cc.preferred DESC, cc.sort_key, cc.id LIMIT 1),
                c.created_at, c.updated_at
         FROM contacts c LEFT JOIN companies co ON co.id = c.company_id
         WHERE c.archived_at IS NULL
         ORDER BY c.display_name, c.id",
    )?;
    // Rows are collected before writing; contractor-scale contact books are
    // small enough that double-buffering the file is not worth streaming.
    let rows = statement
        .query_map([], |row| {
            let favorite: bool = row.get(15)?;
            let mut cells = Vec::with_capacity(headers.len());
            for index in 0..15 {
                cells.push(row.get::<_, Option<String>>(index)?.unwrap_or_default());
            }
            cells.push(favorite.to_string());
            for index in 16..19 {
                cells.push(row.get::<_, Option<String>>(index)?.unwrap_or_default());
            }
            Ok((
                row.get::<_, String>(0)?,
                cells,
                row.get::<_, String>(19)?,
                row.get::<_, String>(20)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);

    let tags = export_tags(connection, "contact")?;
    write_export_record(writer, &headers)?;
    let row_count = rows.len();
    for (id, mut cells, created_at, updated_at) in rows {
        cells.push(tags.get(&id).cloned().unwrap_or_default());
        for (definition_id, _) in &definitions {
            cells.push(
                values
                    .get(&(id.clone(), definition_id.clone()))
                    .cloned()
                    .unwrap_or_default(),
            );
        }
        cells.push(created_at);
        cells.push(updated_at);
        write_export_record(writer, &cells)?;
    }
    Ok(row_count)
}

/// Write every active opportunity to a CSV file; money is exported in major
/// units with a separate currency column.
pub fn export_opportunities_csv(
    storage: &mut Storage,
    path: &str,
    overwrite: bool,
) -> Result<CsvExportReport, ApplicationError> {
    let path = check_export_destination(storage, path, overwrite)?;
    let mut writer = csv::Writer::from_writer(Vec::new());
    let row_count = write_opportunities_csv(storage.connection(), &mut writer)?;
    write_export_file(&path, &csv_bytes(writer)?)?;
    log_export(storage, "opportunities", row_count, &path)?;
    Ok(CsvExportReport { path, row_count })
}

/// Opportunity CSV body, shared by the file export and the portable archive.
pub(crate) fn write_opportunities_csv<W: std::io::Write>(
    connection: &rusqlite::Connection,
    writer: &mut csv::Writer<W>,
) -> Result<usize, ApplicationError> {
    let definitions = export_custom_field_defs(connection, "opportunity")?;
    let values = export_custom_field_values(connection, "opportunity")?;

    let mut headers = vec![
        "id",
        "name",
        "contact_display_name",
        "company",
        "stage",
        "value",
        "currency_code",
        "probability_percent",
        "expected_close_date",
        "source",
        "source_label",
        "tags",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    headers.extend(definitions.iter().map(|(_, label)| label.clone()));
    headers.push("created_at".into());
    headers.push("updated_at".into());

    let mut statement = connection.prepare(
        "SELECT o.id, o.name, c.display_name, co.name, s.name, o.value_minor, o.currency_code,
                o.probability_percent, o.expected_close_date, o.source, o.source_label,
                o.created_at, o.updated_at
         FROM opportunities o
         JOIN stages s ON s.id = o.stage_id
         LEFT JOIN contacts c ON c.id = o.contact_id
         LEFT JOIN companies co ON co.id = o.company_id
         WHERE o.archived_at IS NULL
         ORDER BY o.name, o.id",
    )?;
    let rows = statement
        .query_map([], |row| {
            let value_minor: i64 = row.get(5)?;
            let probability: Option<i64> = row.get(7)?;
            let cells = vec![
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                row.get::<_, String>(4)?,
                format!("{:.2}", value_minor as f64 / 100.0),
                row.get::<_, String>(6)?,
                probability
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
                row.get::<_, Option<String>>(8)?.unwrap_or_default(),
                row.get::<_, Option<String>>(9)?.unwrap_or_default(),
                row.get::<_, Option<String>>(10)?.unwrap_or_default(),
            ];
            Ok((
                row.get::<_, String>(0)?,
                cells,
                row.get::<_, String>(11)?,
                row.get::<_, String>(12)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);

    let tags = export_tags(connection, "opportunity")?;
    write_export_record(writer, &headers)?;
    let row_count = rows.len();
    for (id, mut cells, created_at, updated_at) in rows {
        cells.push(tags.get(&id).cloned().unwrap_or_default());
        for (definition_id, _) in &definitions {
            cells.push(
                values
                    .get(&(id.clone(), definition_id.clone()))
                    .cloned()
                    .unwrap_or_default(),
            );
        }
        cells.push(created_at);
        cells.push(updated_at);
        write_export_record(writer, &cells)?;
    }
    Ok(row_count)
}

/// Effective column positions for the mapped targets, resolved once per file.
type MappingIndexes = std::collections::BTreeMap<&'static str, usize>;

/// A parsed CSV file: header names plus each data record with its line number.
type CsvFile = (Vec<String>, Vec<(u64, csv::StringRecord)>);

/// Malformed or non-UTF-8 files are caller errors, not storage failures, so
/// they surface as invalid input with a message a user can act on.
fn csv_parse_error(path: &str, error: csv::Error) -> ApplicationError {
    let message = match error.kind() {
        csv::ErrorKind::Utf8 { .. } => format!(
            "\"{path}\" is not UTF-8 text; re-save it as CSV UTF-8 \
             (Excel: \"CSV UTF-8 (Comma delimited)\")"
        ),
        _ => format!("cannot read \"{path}\": {error}"),
    };
    ApplicationError::InvalidInput {
        field: "path".into(),
        message,
    }
}

/// Read a CSV file into its headers plus every data record with its line
/// number. Ragged rows are tolerated; missing cells read as empty. The whole
/// file is buffered — contractor-scale contact lists make streaming needless.
fn read_csv_records(path: &str) -> Result<CsvFile, ApplicationError> {
    let path = required_text("path", path.to_owned(), 4096)?;
    let file = std::fs::File::open(&path).map_err(|error| ApplicationError::InvalidInput {
        field: "path".into(),
        message: format!("cannot open \"{path}\": {error}"),
    })?;
    let mut reader = csv::ReaderBuilder::new().flexible(true).from_reader(file);
    let mut headers = reader
        .headers()
        .map_err(|error| csv_parse_error(&path, error))?
        .iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    // A hand-edited "Name,Email," keeps a trailing empty column; drop those
    // (and the cells under them) rather than refusing the file.
    while headers
        .last()
        .is_some_and(|header| header.trim().is_empty())
    {
        headers.pop();
    }
    validate_csv_headers(&headers)?;
    let mut records = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|error| csv_parse_error(&path, error))?;
        // Skip rows that are entirely blank — trailing newlines are common.
        if record.iter().all(|cell| cell.trim().is_empty()) {
            continue;
        }
        let line = record.position().map(csv::Position::line).unwrap_or(0);
        records.push((line, record));
    }
    Ok((headers, records))
}

/// Headers must be present and unique once trailing empty columns are dropped:
/// mapping by name silently loses data otherwise, because only the first
/// column of a repeated name is ever read.
fn validate_csv_headers(headers: &[String]) -> Result<(), ApplicationError> {
    if headers.is_empty() {
        return Err(ApplicationError::InvalidInput {
            field: "path".into(),
            message: "the file has no header row".into(),
        });
    }
    for (index, header) in headers.iter().enumerate() {
        if header.trim().is_empty() {
            return Err(ApplicationError::InvalidInput {
                field: "path".into(),
                message: format!("column {} has an empty header", index + 1),
            });
        }
        if headers[..index].iter().any(|earlier| earlier == header) {
            return Err(ApplicationError::InvalidInput {
                field: "path".into(),
                message: format!("duplicate column header \"{header}\"; make headers unique"),
            });
        }
    }
    Ok(())
}

/// Normalize a header for fuzzy matching: lowercase, alphanumerics only.
fn normalize_header(header: &str) -> String {
    header
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Guess a mapping from the file's headers; each header is claimed by at most
/// one target and the first matching alias wins.
fn guess_contact_mapping(headers: &[String]) -> ContactImportMapping {
    let normalized = headers
        .iter()
        .map(|header| normalize_header(header))
        .collect::<Vec<_>>();
    let mut claimed = vec![false; headers.len()];
    let mut mapping = ContactImportMapping::default();
    for (target, aliases) in IMPORT_TARGETS {
        'target: for alias in *aliases {
            for (index, candidate) in normalized.iter().enumerate() {
                if !claimed[index] && candidate == alias {
                    mapping.set(target, headers[index].clone());
                    claimed[index] = true;
                    break 'target;
                }
            }
        }
    }
    mapping
}

/// Resolve mapped header names to column positions; an unknown header is a
/// caller error, not a per-row problem.
fn mapping_indexes(
    headers: &[String],
    mapping: &ContactImportMapping,
) -> Result<MappingIndexes, ApplicationError> {
    let mut indexes = MappingIndexes::new();
    for (target, _) in IMPORT_TARGETS {
        let Some(header) = mapping.header_for(target) else {
            continue;
        };
        let index = headers
            .iter()
            .position(|candidate| candidate == header)
            .ok_or_else(|| ApplicationError::InvalidInput {
                field: format!("mapping.{target}"),
                message: format!("column \"{header}\" is not in the file"),
            })?;
        indexes.insert(target, index);
    }
    Ok(indexes)
}

/// Trimmed cell for a mapped target, or None when unmapped or blank. The
/// export formula guard is undone here so our own files round-trip byte for
/// byte instead of storing "\'+1 555 0100".
fn mapped_cell(
    record: &csv::StringRecord,
    indexes: &MappingIndexes,
    target: &str,
) -> Option<String> {
    let index = *indexes.get(target)?;
    let value = unescape_formula_guard(record.get(index)?).trim();
    (!value.is_empty()).then(|| value.to_owned())
}

/// Strip exactly one leading quote that only exists to defuse a formula.
fn unescape_formula_guard(value: &str) -> &str {
    let mut characters = value.chars();
    if characters.next() != Some('\'') {
        return value;
    }
    match characters.next() {
        Some(next) if is_formula_trigger(next) => &value[1..],
        _ => value,
    }
}

fn is_formula_trigger(character: char) -> bool {
    FORMULA_PREFIXES.contains(&character) || character == '\r'
}

/// Validate one CSV row into contact fields, reusing the shared contact
/// validation so imports cannot drift from interactive writes.
fn prepare_import_row(
    line: u64,
    record: &csv::StringRecord,
    indexes: &MappingIndexes,
) -> Result<PreparedImportRow, ContactImportIssue> {
    let issue = |error: ApplicationError| ContactImportIssue {
        line,
        reason: error.to_string(),
    };
    let mut channels = Vec::new();
    if let Some(email) = mapped_cell(record, indexes, "email") {
        channels.push(ChannelInput {
            kind: "email".into(),
            label: None,
            value: email,
            preferred: true,
        });
    }
    if let Some(phone) = mapped_cell(record, indexes, "phone") {
        channels.push(ChannelInput {
            kind: "phone".into(),
            label: None,
            value: phone,
            preferred: true,
        });
    }
    let kind = mapped_cell(record, indexes, "kind");
    let kind_mapped = kind.is_some();
    let patch = ContactPatch {
        company_id: None, // resolved by name while applying
        first_name: mapped_cell(record, indexes, "firstName"),
        last_name: mapped_cell(record, indexes, "lastName"),
        display_name: mapped_cell(record, indexes, "displayName"),
        role: mapped_cell(record, indexes, "role"),
        kind: kind.unwrap_or_else(|| IMPORT_DEFAULT_KIND.to_owned()),
        preferred_contact_method: mapped_cell(record, indexes, "preferredContactMethod"),
        address_line1: mapped_cell(record, indexes, "addressLine1"),
        address_line2: mapped_cell(record, indexes, "addressLine2"),
        city: mapped_cell(record, indexes, "city"),
        state: mapped_cell(record, indexes, "state"),
        postal_code: mapped_cell(record, indexes, "postalCode"),
        property_type: mapped_cell(record, indexes, "propertyType"),
        notes: mapped_cell(record, indexes, "notes"),
        favorite: false,
        channels,
    };
    let fields = validate_contact_patch(patch.clone()).map_err(issue)?;
    let external_id = match mapped_cell(record, indexes, "externalId") {
        None => None,
        Some(value) => Some(required_text("externalId", value, 200).map_err(issue)?),
    };
    let company_name = match mapped_cell(record, indexes, "company") {
        None => None,
        Some(value) => Some(required_text("company", value, 200).map_err(issue)?),
    };
    let mut tags = Vec::new();
    if let Some(value) = mapped_cell(record, indexes, "tags") {
        for label in value.split(';') {
            let label = label.trim();
            if label.is_empty() {
                continue;
            }
            let label = required_text("tags", label.to_owned(), 80).map_err(issue)?;
            if !tags.iter().any(|existing| existing == &label) {
                tags.push(label);
            }
        }
    }
    Ok(PreparedImportRow {
        line,
        patch,
        kind_mapped,
        fields,
        external_id,
        company_name,
        tags,
    })
}

/// Find a company by exact name (case-insensitive), creating it when missing.
fn resolve_import_company(
    transaction: &Transaction<'_>,
    name: &str,
    actor: Actor,
) -> Result<String, ApplicationError> {
    let existing: Option<String> = transaction
        .query_row(
            "SELECT id FROM companies WHERE trim(name) = trim(?1) COLLATE NOCASE
             ORDER BY archived_at IS NOT NULL, created_at, id LIMIT 1",
            [name],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = existing {
        return Ok(id);
    }
    let id = new_id();
    let now = now_utc();
    transaction.execute(
        "INSERT INTO companies (id, name, kind, created_at, updated_at, version)
         VALUES (?1, ?2, ?3, ?4, ?4, 1)",
        params![id, name, IMPORT_DEFAULT_KIND, now],
    )?;
    refresh_search_projection(transaction, "company", &id)?;
    log_command(
        transaction,
        actor,
        "company",
        &id,
        &format!("imported company \"{name}\""),
    )?;
    Ok(id)
}

/// Match an incoming row to an existing contact: external id first, then the
/// record id so an exported file re-imports onto the same contacts.
fn find_contact_for_import(
    transaction: &Transaction<'_>,
    external_id: Option<&str>,
) -> Result<Option<ImportMatch>, ApplicationError> {
    let Some(external_id) = external_id else {
        return Ok(None);
    };
    let matched = transaction
        .query_row(
            "SELECT id, archived_at IS NOT NULL FROM contacts
             WHERE external_id = ?1 OR id = ?1
             ORDER BY external_id = ?1 DESC LIMIT 1",
            [external_id],
            |row| {
                Ok(ImportMatch {
                    id: row.get(0)?,
                    archived: row.get(1)?,
                })
            },
        )
        .optional()?;
    Ok(matched)
}

/// Insert one imported contact with its channels, projection, and log row.
fn insert_imported_contact(
    transaction: &Transaction<'_>,
    fields: &ValidContactFields,
    external_id: Option<&str>,
    actor: Actor,
) -> Result<String, ApplicationError> {
    let contact_id = new_id();
    let now = now_utc();
    transaction.execute(
        "INSERT INTO contacts (
            id, company_id, first_name, last_name, display_name, role, kind,
            preferred_contact_method, address_line1, address_line2, city, state,
            postal_code, property_type, notes, favorite, external_id,
            archived_at, created_at, updated_at, version
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                   ?15, ?16, ?17, NULL, ?18, ?18, 1)",
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
            external_id,
            now,
        ],
    )?;
    insert_channels(transaction, &contact_id, &fields.channels)?;
    refresh_search_projection(transaction, "contact", &contact_id)?;
    log_command(
        transaction,
        actor,
        "contact",
        &contact_id,
        &format!("imported contact \"{}\"", fields.display_name),
    )?;
    Ok(contact_id)
}

/// Patch a matched contact: only the columns the file carried are written, the
/// version bumps like any other write, and channels are added, never removed.
fn update_imported_contact(
    transaction: &Transaction<'_>,
    contact_id: &str,
    row: &PreparedImportRow,
    company_id: Option<String>,
    actor: Actor,
) -> Result<(), ApplicationError> {
    let existing = require_contact(transaction, contact_id)?;
    let merged = merge_import_patch(&existing, row, company_id);
    let fields = validate_contact_patch(merged)?;
    transaction.execute(
        "UPDATE contacts SET
            company_id = ?2, first_name = ?3, last_name = ?4, display_name = ?5,
            role = ?6, kind = ?7, preferred_contact_method = ?8,
            address_line1 = ?9, address_line2 = ?10, city = ?11, state = ?12,
            postal_code = ?13, property_type = ?14, notes = ?15,
            updated_at = ?16, version = ?17
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
            now_utc(),
            existing.version + 1,
        ],
    )?;
    add_import_channels(transaction, &existing.id, &row.fields.channels)?;
    refresh_search_projection(transaction, "contact", &existing.id)?;
    log_command(
        transaction,
        actor,
        "contact",
        &existing.id,
        &format!("imported update to contact \"{}\"", fields.display_name),
    )?;
    Ok(())
}

/// Overlay the row's mapped cells on the stored contact. Unmapped and blank
/// cells keep the stored value — v1 imports never clear a field.
fn merge_import_patch(
    existing: &Contact,
    row: &PreparedImportRow,
    company_id: Option<String>,
) -> ContactPatch {
    let patch = &row.patch;
    let first_name = patch
        .first_name
        .clone()
        .or_else(|| existing.first_name.clone());
    let last_name = patch
        .last_name
        .clone()
        .or_else(|| existing.last_name.clone());
    // A name column re-derives the display name only when the stored one was
    // itself derived from the stored name parts; a curated display name is
    // never overwritten by a first/last-name column.
    let stored_was_derived = derive_display_name(None, &existing.first_name, &existing.last_name)
        .is_ok_and(|derived| derived == existing.display_name);
    let display_name = match &patch.display_name {
        Some(value) => value.clone(),
        None if stored_was_derived && (patch.first_name.is_some() || patch.last_name.is_some()) => {
            derive_display_name(None, &first_name, &last_name)
                .unwrap_or_else(|_| existing.display_name.clone())
        }
        None => existing.display_name.clone(),
    };
    ContactPatch {
        company_id: company_id.or_else(|| existing.company_id.clone()),
        first_name,
        last_name,
        display_name: Some(display_name),
        role: patch.role.clone().or_else(|| {
            existing
                .role
                .map(|role| role.as_database_value().to_owned())
        }),
        kind: if row.kind_mapped {
            patch.kind.clone()
        } else {
            existing.kind.as_database_value().to_owned()
        },
        preferred_contact_method: patch
            .preferred_contact_method
            .clone()
            .or_else(|| existing.preferred_contact_method.clone()),
        address_line1: patch
            .address_line1
            .clone()
            .or_else(|| existing.address_line1.clone()),
        address_line2: patch
            .address_line2
            .clone()
            .or_else(|| existing.address_line2.clone()),
        city: patch.city.clone().or_else(|| existing.city.clone()),
        state: patch.state.clone().or_else(|| existing.state.clone()),
        postal_code: patch
            .postal_code
            .clone()
            .or_else(|| existing.postal_code.clone()),
        property_type: patch
            .property_type
            .clone()
            .or_else(|| existing.property_type.clone()),
        notes: patch.notes.clone().or_else(|| existing.notes.clone()),
        favorite: existing.favorite,
        // Channels are applied additively, outside the patch.
        channels: Vec::new(),
    }
}

/// Add the row's channels to a contact. An exact value already on the record
/// is left alone and nothing is ever deleted, so secondary phones and emails
/// survive repeated imports.
fn add_import_channels(
    transaction: &Transaction<'_>,
    contact_id: &str,
    channels: &[ValidChannel],
) -> Result<(), ApplicationError> {
    for channel in channels {
        let kind = channel.kind.as_database_value();
        let exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM contact_channels
             WHERE contact_id = ?1 AND kind = ?2 AND value = ?3)",
            params![contact_id, kind, channel.value],
            |row| row.get(0),
        )?;
        if exists {
            continue;
        }
        // Only the first channel of a kind claims "preferred"; an existing
        // preferred phone or email keeps that status.
        let (kind_count, next_sort_key): (i64, i64) = transaction.query_row(
            "SELECT COUNT(*), COALESCE(MAX(sort_key), -1) + 1 FROM contact_channels
             WHERE contact_id = ?1 AND kind = ?2",
            params![contact_id, kind],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        transaction.execute(
            "INSERT INTO contact_channels (id, contact_id, kind, label, value, preferred, sort_key)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                new_id(),
                contact_id,
                kind,
                channel.label,
                channel.value,
                kind_count == 0,
                next_sort_key,
            ],
        )?;
    }
    Ok(())
}

/// Add the row's tags to a contact, creating tags that do not exist yet.
/// Existing tags on the contact are kept — imports add, never remove.
fn apply_import_tags(
    transaction: &Transaction<'_>,
    contact_id: &str,
    labels: &[String],
    actor: Actor,
) -> Result<(), ApplicationError> {
    for label in labels {
        let existing: Option<String> = transaction
            .query_row(
                "SELECT id FROM tags WHERE label = ?1 COLLATE NOCASE",
                [label],
                |row| row.get(0),
            )
            .optional()?;
        let tag_id = match existing {
            Some(id) => id,
            None => {
                let count: i64 =
                    transaction.query_row("SELECT COUNT(*) FROM tags", [], |row| row.get(0))?;
                if count >= MAX_TAGS {
                    return Err(limit_error("tag_limit_reached", "tags", MAX_TAGS));
                }
                let id = new_id();
                let now = now_utc();
                transaction.execute(
                    "INSERT INTO tags (id, label, color_role, created_at, updated_at, version)
                     VALUES (?1, ?2, NULL, ?3, ?3, 1)",
                    params![id, label, now],
                )?;
                log_command(transaction, actor, "tag", &id, "imported tag")?;
                id
            }
        };
        transaction.execute(
            "INSERT OR IGNORE INTO record_tags (tag_id, entity_type, record_id, created_at)
             VALUES (?1, 'contact', ?2, ?3)",
            params![tag_id, contact_id, now_utc()],
        )?;
    }
    Ok(())
}

/// Active custom field definitions for an entity type as (id, label) in the
/// order their columns appear in an export.
fn export_custom_field_defs(
    connection: &rusqlite::Connection,
    entity_type: &str,
) -> Result<Vec<(String, String)>, ApplicationError> {
    let mut statement = connection.prepare(
        "SELECT id, label FROM custom_field_defs
         WHERE entity_type = ?1 AND archived_at IS NULL ORDER BY sort_key, id",
    )?;
    let rows = statement
        .query_map([entity_type], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Custom field values keyed by (record id, definition id), already rendered
/// as export text.
fn export_custom_field_values(
    connection: &rusqlite::Connection,
    entity_type: &str,
) -> Result<std::collections::HashMap<(String, String), String>, ApplicationError> {
    let mut statement = connection.prepare(
        "SELECT v.record_id, v.definition_id, v.text_value, v.number_value, v.date_value, o.label
         FROM custom_field_values v
         LEFT JOIN custom_field_options o ON o.id = v.option_id
         WHERE v.entity_type = ?1",
    )?;
    let rows = statement
        .query_map([entity_type], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<f64>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .map(
            |(record_id, definition_id, text, number, date, option_label)| {
                let value = text
                    .or_else(|| number.map(format_export_number))
                    .or(date)
                    .or(option_label)
                    .unwrap_or_default();
                ((record_id, definition_id), value)
            },
        )
        .collect())
}

/// Semicolon-joined tag labels per record, keyed by record id.
fn export_tags(
    connection: &rusqlite::Connection,
    entity_type: &str,
) -> Result<std::collections::HashMap<String, String>, ApplicationError> {
    let mut statement = connection.prepare(
        "SELECT rt.record_id, t.label FROM record_tags rt JOIN tags t ON t.id = rt.tag_id
         WHERE rt.entity_type = ?1 ORDER BY rt.record_id, t.label COLLATE NOCASE",
    )?;
    let rows = statement
        .query_map([entity_type], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut grouped: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for (record_id, label) in rows {
        let entry = grouped.entry(record_id).or_default();
        if !entry.is_empty() {
            entry.push(';');
        }
        entry.push_str(&label);
    }
    Ok(grouped)
}

/// Render a custom field number without a trailing ".0" for whole values.
fn format_export_number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{value:.0}")
    } else {
        format!("{value}")
    }
}

/// Validate an export destination the way `export_handoff_envelope` does:
/// existing files are only replaced when the caller asks for it, and the live
/// database and its sidecars are never a destination — exporting onto
/// contractorcrm.sqlite3 destroys the database it is exporting.
pub(crate) fn check_export_destination(
    storage: &Storage,
    path: &str,
    overwrite: bool,
) -> Result<String, ApplicationError> {
    let path = required_text("path", path.to_owned(), 4096)?;
    let destination = std::path::Path::new(&path);
    if is_database_file(storage.database_path(), destination) {
        return Err(ApplicationError::ValidationFailed {
            code: "destination_is_database",
            field: "path".into(),
            message: format!("{path} belongs to the live database; pick another destination"),
        });
    }
    if destination.exists() && !overwrite {
        return Err(ApplicationError::ValidationFailed {
            code: "destination_exists",
            field: "path".into(),
            message: format!("{path} already exists; enable overwrite to replace it"),
        });
    }
    Ok(path)
}

/// True when `destination` is the database file, one of its WAL/SHM sidecars,
/// or one of the `<database>.*.bak` safety copies. Directories are compared
/// canonically so "./data/../data/crm.sqlite3" cannot slip through; a
/// destination whose directory does not exist yet can never be the database.
fn is_database_file(database_path: &std::path::Path, destination: &std::path::Path) -> bool {
    let resolve = |path: &std::path::Path| {
        let parent = match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
            _ => std::path::PathBuf::from("."),
        };
        std::fs::canonicalize(parent).ok()
    };
    let (Some(database_directory), Some(destination_directory)) =
        (resolve(database_path), resolve(destination))
    else {
        return false;
    };
    if database_directory != destination_directory {
        return false;
    }
    let (Some(database_name), Some(destination_name)) = (
        database_path.file_name().and_then(|name| name.to_str()),
        destination.file_name().and_then(|name| name.to_str()),
    ) else {
        return false;
    };
    destination_name == database_name
        || destination_name == format!("{database_name}-wal")
        || destination_name == format!("{database_name}-shm")
        || destination_name.starts_with(&format!("{database_name}."))
}

/// Record the export in the command log, mirroring backup/hand-off exports.
fn log_export(
    storage: &mut Storage,
    entity_id: &str,
    row_count: usize,
    path: &str,
) -> Result<(), ApplicationError> {
    let transaction = immediate(storage)?;
    log_command(
        &transaction,
        Actor::User,
        "export",
        entity_id,
        &format!("exported {row_count} {entity_id} to \"{path}\""),
    )?;
    transaction.commit()?;
    Ok(())
}

/// Neutralize spreadsheet formulas: a cell starting with a formula trigger is
/// prefixed with a single quote so Excel and Sheets treat it as text.
fn sanitize_export_cell(value: &str) -> String {
    match value.chars().next() {
        Some(first) if is_formula_trigger(first) => format!("'{value}"),
        _ => value.to_owned(),
    }
}

fn write_export_record<W: std::io::Write>(
    writer: &mut csv::Writer<W>,
    cells: &[String],
) -> Result<(), ApplicationError> {
    writer
        .write_record(cells.iter().map(|cell| sanitize_export_cell(cell)))
        .map_err(|error| ApplicationError::Io(std::io::Error::other(error.to_string())))
}

/// Finished CSV bytes; `into_inner` flushes the buffered writer.
pub(crate) fn csv_bytes(writer: csv::Writer<Vec<u8>>) -> Result<Vec<u8>, ApplicationError> {
    writer
        .into_inner()
        .map_err(|error| ApplicationError::Io(std::io::Error::other(error.to_string())))
}

/// Write a rendered export to `path`, creating missing parent directories.
pub(crate) fn write_export_file(path: &str, bytes: &[u8]) -> Result<(), ApplicationError> {
    let destination = std::path::Path::new(path);
    if let Some(parent) = destination.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(destination, bytes)?;
    Ok(())
}
