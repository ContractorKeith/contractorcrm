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
    pub archived_at: Option<String>,
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
