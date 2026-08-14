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
