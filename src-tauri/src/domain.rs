use serde::{Deserialize, Serialize};

/// Party classification shared by companies and contacts (docs/DATA_MODEL.md).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PartyKind {
    Client,
    Lead,
    Sub,
    Vendor,
    Supplier,
    Other,
}

impl PartyKind {
    pub(crate) fn as_database_value(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Lead => "lead",
            Self::Sub => "sub",
            Self::Vendor => "vendor",
            Self::Supplier => "supplier",
            Self::Other => "other",
        }
    }

    pub(crate) fn from_database_value(value: &str) -> Option<Self> {
        match value {
            "client" => Some(Self::Client),
            "lead" => Some(Self::Lead),
            "sub" => Some(Self::Sub),
            "vendor" => Some(Self::Vendor),
            "supplier" => Some(Self::Supplier),
            "other" => Some(Self::Other),
            _ => None,
        }
    }
}

/// What a contact does for their company or property.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContactRole {
    Owner,
    Estimator,
    SiteContact,
    Office,
    Other,
}

impl ContactRole {
    pub(crate) fn as_database_value(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Estimator => "estimator",
            Self::SiteContact => "site_contact",
            Self::Office => "office",
            Self::Other => "other",
        }
    }

    pub(crate) fn from_database_value(value: &str) -> Option<Self> {
        match value {
            "owner" => Some(Self::Owner),
            "estimator" => Some(Self::Estimator),
            "site_contact" => Some(Self::SiteContact),
            "office" => Some(Self::Office),
            "other" => Some(Self::Other),
            _ => None,
        }
    }
}

/// Typed multi-value contact channel kind — phones and emails in v1.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    Phone,
    Email,
}

impl ChannelKind {
    pub(crate) fn as_database_value(self) -> &'static str {
        match self {
            Self::Phone => "phone",
            Self::Email => "email",
        }
    }

    pub(crate) fn from_database_value(value: &str) -> Option<Self> {
        match value {
            "phone" => Some(Self::Phone),
            "email" => Some(Self::Email),
            _ => None,
        }
    }
}

/// Who performed a mutation; every command_log row records one.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Actor {
    #[default]
    User,
    Agent,
    Import,
}

impl Actor {
    pub(crate) fn as_database_value(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Agent => "agent",
            Self::Import => "import",
        }
    }

    pub(crate) fn from_database_value(value: &str) -> Option<Self> {
        match value {
            "user" => Some(Self::User),
            "agent" => Some(Self::Agent),
            "import" => Some(Self::Import),
            _ => None,
        }
    }
}

/// A company — client, sub, vendor, or supplier grouping contacts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Company {
    pub id: String,
    pub name: String,
    pub kind: PartyKind,
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
    pub archived_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub version: i64,
}

/// A person; channels are always loaded with the contact.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Contact {
    pub id: String,
    pub company_id: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub display_name: String,
    pub role: Option<ContactRole>,
    pub kind: PartyKind,
    pub preferred_contact_method: Option<String>,
    pub address_line1: Option<String>,
    pub address_line2: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub postal_code: Option<String>,
    pub property_type: Option<String>,
    pub notes: Option<String>,
    pub favorite: bool,
    pub archived_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub version: i64,
    pub channels: Vec<ContactChannel>,
}

/// One phone or email row belonging to a contact.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactChannel {
    pub id: String,
    pub contact_id: String,
    pub kind: ChannelKind,
    pub label: Option<String>,
    pub value: String,
    pub preferred: bool,
    pub sort_key: i64,
}

/// What a pipeline stage means for an opportunity sitting in it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StageKind {
    Open,
    Won,
    Lost,
}

impl StageKind {
    pub(crate) fn from_database_value(value: &str) -> Option<Self> {
        match value {
            "open" => Some(Self::Open),
            "won" => Some(Self::Won),
            "lost" => Some(Self::Lost),
            _ => None,
        }
    }
}

/// Where an opportunity came from (docs/DATA_MODEL.md source enum).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OpportunitySource {
    Referral,
    RepeatClient,
    Website,
    Sign,
    Other,
}

impl OpportunitySource {
    pub(crate) fn as_database_value(self) -> &'static str {
        match self {
            Self::Referral => "referral",
            Self::RepeatClient => "repeat_client",
            Self::Website => "website",
            Self::Sign => "sign",
            Self::Other => "other",
        }
    }

    pub(crate) fn from_database_value(value: &str) -> Option<Self> {
        match value {
            "referral" => Some(Self::Referral),
            "repeat_client" => Some(Self::RepeatClient),
            "website" => Some(Self::Website),
            "sign" => Some(Self::Sign),
            "other" => Some(Self::Other),
            _ => None,
        }
    }
}

/// One user-editable pipeline step; renaming/reordering never rewrites history.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage {
    pub id: String,
    pub pipeline_id: String,
    pub name: String,
    pub sort_key: i64,
    pub kind: StageKind,
    pub created_at: String,
    pub updated_at: String,
    pub version: i64,
}

/// A user-editable reason an opportunity was lost.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LostReason {
    pub id: String,
    pub label: String,
    pub sort_key: i64,
    pub active: bool,
}

/// Money as integer minor units plus ISO currency code — no floats anywhere.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Money {
    pub value_minor: i64,
    pub currency_code: String,
}

/// Reference to a record in another tool — a quote or a ContractorProject
/// job — stored on the opportunity once the hand-off happens.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HandoffRef {
    /// Which external tool owns the record, e.g. "contractorproject".
    pub tool: String,
    /// The record's id inside that tool.
    pub external_id: String,
    /// Human-readable label, e.g. "Q-123".
    pub label: Option<String>,
    pub linked_at: String,
}

/// Potential work moving through the pipeline toward won or lost.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Opportunity {
    pub id: String,
    pub name: String,
    pub contact_id: Option<String>,
    pub company_id: Option<String>,
    pub stage_id: String,
    pub value: Money,
    pub probability_percent: Option<i64>,
    pub expected_close_date: Option<String>,
    pub source: Option<OpportunitySource>,
    pub source_label: Option<String>,
    pub lost_reason_id: Option<String>,
    pub notes: Option<String>,
    pub quote_ref: Option<HandoffRef>,
    pub job_ref: Option<HandoffRef>,
    pub archived_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub version: i64,
}

/// Which record type an activity hangs off — its polymorphic parent.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParentType {
    Contact,
    Company,
    Opportunity,
}

impl ParentType {
    pub(crate) fn as_database_value(self) -> &'static str {
        match self {
            Self::Contact => "contact",
            Self::Company => "company",
            Self::Opportunity => "opportunity",
        }
    }

    pub(crate) fn from_database_value(value: &str) -> Option<Self> {
        match value {
            "contact" => Some(Self::Contact),
            "company" => Some(Self::Company),
            "opportunity" => Some(Self::Opportunity),
            _ => None,
        }
    }
}

/// What kind of touch an activity records.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityKind {
    Call,
    Email,
    Text,
    SiteVisit,
    Meeting,
    Note,
}

impl ActivityKind {
    pub(crate) fn as_database_value(self) -> &'static str {
        match self {
            Self::Call => "call",
            Self::Email => "email",
            Self::Text => "text",
            Self::SiteVisit => "site_visit",
            Self::Meeting => "meeting",
            Self::Note => "note",
        }
    }

    pub(crate) fn from_database_value(value: &str) -> Option<Self> {
        match value {
            "call" => Some(Self::Call),
            "email" => Some(Self::Email),
            "text" => Some(Self::Text),
            "site_visit" => Some(Self::SiteVisit),
            "meeting" => Some(Self::Meeting),
            "note" => Some(Self::Note),
            _ => None,
        }
    }
}

/// Which way a communication went; `none` for notes and on-site touches.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityDirection {
    Inbound,
    Outbound,
    #[default]
    None,
}

impl ActivityDirection {
    pub(crate) fn as_database_value(self) -> &'static str {
        match self {
            Self::Inbound => "inbound",
            Self::Outbound => "outbound",
            Self::None => "none",
        }
    }

    pub(crate) fn from_database_value(value: &str) -> Option<Self> {
        match value {
            "inbound" => Some(Self::Inbound),
            "outbound" => Some(Self::Outbound),
            "none" => Some(Self::None),
            _ => None,
        }
    }
}

/// One logged touch on a contact, company, or opportunity. `occurred_at` is
/// user-editable (UTC ISO-8601); timelines sort by it, not created_at.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    pub id: String,
    pub parent_type: ParentType,
    pub parent_id: String,
    pub kind: ActivityKind,
    pub direction: ActivityDirection,
    pub occurred_at: String,
    pub summary: String,
    /// Markdown body, optional.
    pub body: Option<String>,
    pub actor: Actor,
    pub created_at: String,
    pub updated_at: String,
    pub version: i64,
}

/// How urgent a task is.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskPriority {
    Low,
    #[default]
    Normal,
    High,
}

impl TaskPriority {
    pub(crate) fn as_database_value(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Normal => "normal",
            Self::High => "high",
        }
    }

    pub(crate) fn from_database_value(value: &str) -> Option<Self> {
        match value {
            "low" => Some(Self::Low),
            "normal" => Some(Self::Normal),
            "high" => Some(Self::High),
            _ => None,
        }
    }
}

/// Where a task sits in its lifecycle.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    #[default]
    Open,
    Done,
    Dropped,
}

impl TaskStatus {
    pub(crate) fn as_database_value(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Done => "done",
            Self::Dropped => "dropped",
        }
    }

    pub(crate) fn from_database_value(value: &str) -> Option<Self> {
        match value {
            "open" => Some(Self::Open),
            "done" => Some(Self::Done),
            "dropped" => Some(Self::Dropped),
            _ => None,
        }
    }
}

/// A follow-up or to-do, optionally hanging off a contact, company, or
/// opportunity; personal tasks have no parent. Timestamps are UTC ISO-8601.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub title: String,
    /// Markdown body, optional.
    pub body: Option<String>,
    pub parent_type: Option<ParentType>,
    pub parent_id: Option<String>,
    pub due_at: Option<String>,
    pub remind_at: Option<String>,
    pub priority: TaskPriority,
    pub status: TaskStatus,
    pub completed_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub version: i64,
}

/// One append-only stage change; stores stage ids only, never names.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageHistoryEntry {
    pub id: String,
    pub opportunity_id: String,
    pub from_stage_id: Option<String>,
    pub to_stage_id: String,
    pub actor: Actor,
    pub lost_reason_id: Option<String>,
    pub created_at: String,
}
