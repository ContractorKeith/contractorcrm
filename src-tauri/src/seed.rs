//! Deterministic development seeding — builds a realistic, contractor-flavoured
//! database for scale testing.
//!
//! Every record is written THROUGH the application seam (`create_contact`,
//! `create_opportunity`, `log_activity`, …) so the FTS projection, command log,
//! and validation invariants stay exactly as a real user would leave them.
//! No raw `INSERT` for records lives here.
//!
//! Determinism comes from a tiny xorshift64* generator seeded from the caller,
//! so the same `--seed` always produces the same database (ids and timestamps
//! excepted — those come from the seam's clock and UUIDv7 generator).

use crate::application::{
    create_company, create_contact, create_custom_field_def, create_opportunity, create_tag,
    create_task, list_lost_reasons, list_stages, log_activity, set_record_metadata, ActivityPatch,
    ChannelInput, CompanyPatch, ContactPatch, CreateCompanyRequest, CreateContactRequest,
    CreateCustomFieldDefRequest, CreateOpportunityRequest, CreateTagRequest, CreateTaskRequest,
    CustomFieldOptionInput, CustomFieldValueInput, LogActivityRequest, MoveOpportunityStageRequest,
    OpportunityPatch, SavedViewEntityType, SetRecordMetadataRequest, TaskPatch,
};
use crate::domain::{Actor, StageKind};
use crate::error::ApplicationError;
use crate::storage::Storage;

use chrono::{Duration, SecondsFormat, Utc};

/// How much to generate. Everything scales off the contact count so the
/// proportions of a real book of business stay stable at any size.
#[derive(Clone, Copy, Debug)]
pub struct SeedOptions {
    pub contacts: usize,
    pub seed: u64,
}

impl Default for SeedOptions {
    fn default() -> Self {
        Self {
            contacts: 10_000,
            seed: 42,
        }
    }
}

/// What was written, for the CLI's summary line and for tests to assert on.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SeedSummary {
    pub companies: usize,
    pub contacts: usize,
    pub opportunities: usize,
    pub activities: usize,
    pub tasks: usize,
    pub tags: usize,
    pub custom_field_defs: usize,
    pub records_with_metadata: usize,
}

// Proportions relative to the contact count (issue #42): ~1 company per 5
// contacts, ~1 opportunity per 2, ~3 activities per contact, ~1 task per 2.
const COMPANIES_PER_CONTACT: f64 = 0.2;
const OPPORTUNITIES_PER_CONTACT: f64 = 0.5;
const ACTIVITIES_PER_CONTACT: f64 = 3.0;
const TASKS_PER_CONTACT: f64 = 0.5;

/// The first contact is deliberately the "busy" one: it carries a long
/// timeline so timeline reads have a worst case to measure.
const BUSY_CONTACT_ACTIVITIES: usize = 250;

/// One record in every ten gets tags and custom field values.
const METADATA_EVERY: usize = 10;

/// Seed a freshly opened (empty) database. Progress is reported as
/// (phase, done, total) so a CLI can print a live counter.
pub fn seed_database(
    storage: &mut Storage,
    options: &SeedOptions,
    mut progress: impl FnMut(&str, usize, usize),
) -> Result<SeedSummary, ApplicationError> {
    let contacts_total = options.contacts;
    let companies_total = scaled(contacts_total, COMPANIES_PER_CONTACT).max(1);
    let opportunities_total = scaled(contacts_total, OPPORTUNITIES_PER_CONTACT);
    let activities_total = scaled(contacts_total, ACTIVITIES_PER_CONTACT);
    let tasks_total = scaled(contacts_total, TASKS_PER_CONTACT);

    let mut rng = Rng::new(options.seed);
    let mut summary = SeedSummary::default();

    // --- tags and custom fields ------------------------------------------
    let mut tag_ids = Vec::new();
    for label in TAG_LABELS {
        let tag = create_tag(
            storage,
            CreateTagRequest {
                actor: Actor::Import,
                label: (*label).to_owned(),
                color_role: None,
            },
        )?;
        tag_ids.push(tag.id);
    }
    summary.tags = tag_ids.len();

    let contact_text_field = create_custom_field_def(
        storage,
        CreateCustomFieldDefRequest {
            actor: Actor::Import,
            entity_type: SavedViewEntityType::Contact,
            label: "Gate code".to_owned(),
            field_type: "text".to_owned(),
            options: Vec::new(),
        },
    )?;
    let contact_select_field = create_custom_field_def(
        storage,
        CreateCustomFieldDefRequest {
            actor: Actor::Import,
            entity_type: SavedViewEntityType::Contact,
            label: "Preferred crew".to_owned(),
            field_type: "select".to_owned(),
            options: CREW_NAMES
                .iter()
                .map(|label| CustomFieldOptionInput {
                    id: None,
                    label: (*label).to_owned(),
                })
                .collect(),
        },
    )?;
    let opportunity_number_field = create_custom_field_def(
        storage,
        CreateCustomFieldDefRequest {
            actor: Actor::Import,
            entity_type: SavedViewEntityType::Opportunity,
            label: "Linear feet".to_owned(),
            field_type: "number".to_owned(),
            options: Vec::new(),
        },
    )?;
    summary.custom_field_defs = 3;

    // --- companies --------------------------------------------------------
    let mut company_ids = Vec::with_capacity(companies_total);
    for index in 0..companies_total {
        let name = company_name(&mut rng, index);
        let city = pick(CITIES, &mut rng);
        let company = create_company(
            storage,
            CreateCompanyRequest {
                actor: Actor::Import,
                company: CompanyPatch {
                    name,
                    kind: (*pick(COMPANY_KINDS, &mut rng)).to_owned(),
                    phone: Some(phone_number(&mut rng)),
                    email: None,
                    website: None,
                    address_line1: Some(street_address(&mut rng)),
                    address_line2: None,
                    city: Some((*city).to_owned()),
                    state: Some("FL".to_owned()),
                    postal_code: Some(postal_code(&mut rng)),
                    service_area: Some(format!("{city} metro")),
                    license_notes: None,
                    notes: Some((*pick(COMPANY_NOTES, &mut rng)).to_owned()),
                },
            },
        )?;
        company_ids.push(company.id);
        if index % 250 == 0 {
            progress("companies", index, companies_total);
        }
    }
    summary.companies = company_ids.len();
    progress("companies", companies_total, companies_total);

    // --- contacts ---------------------------------------------------------
    let mut contact_ids = Vec::with_capacity(contacts_total);
    for index in 0..contacts_total {
        // Roughly two thirds of contacts belong to a company; the rest are
        // homeowners with no company on file.
        let company_id = if rng.below(3) > 0 {
            Some(company_ids[rng.below(company_ids.len() as u64) as usize].clone())
        } else {
            None
        };
        let first_name = (*pick(FIRST_NAMES, &mut rng)).to_owned();
        let last_name = (*pick(LAST_NAMES, &mut rng)).to_owned();
        let city = pick(CITIES, &mut rng);
        let contact = create_contact(
            storage,
            CreateContactRequest {
                actor: Actor::Import,
                contact: ContactPatch {
                    company_id,
                    first_name: Some(first_name.clone()),
                    // Suffix keeps display names unique enough to search for.
                    last_name: Some(format!("{last_name}-{index}")),
                    display_name: None,
                    role: Some((*pick(CONTACT_ROLES, &mut rng)).to_owned()),
                    kind: (*pick(CONTACT_KINDS, &mut rng)).to_owned(),
                    preferred_contact_method: None,
                    address_line1: Some(street_address(&mut rng)),
                    address_line2: None,
                    city: Some((*city).to_owned()),
                    state: Some("FL".to_owned()),
                    postal_code: Some(postal_code(&mut rng)),
                    property_type: Some((*pick(PROPERTY_TYPES, &mut rng)).to_owned()),
                    notes: Some((*pick(CONTACT_NOTES, &mut rng)).to_owned()),
                    favorite: index % 200 == 0,
                    channels: vec![
                        ChannelInput {
                            kind: "phone".to_owned(),
                            label: Some("mobile".to_owned()),
                            value: phone_number(&mut rng),
                            preferred: true,
                        },
                        ChannelInput {
                            kind: "email".to_owned(),
                            label: Some("work".to_owned()),
                            value: format!(
                                "{}.{}{}@example.com",
                                first_name.to_ascii_lowercase(),
                                last_name.to_ascii_lowercase(),
                                index
                            ),
                            preferred: false,
                        },
                    ],
                },
            },
        )?;
        contact_ids.push(contact.id);
        if index % 500 == 0 {
            progress("contacts", index, contacts_total);
        }
    }
    summary.contacts = contact_ids.len();
    progress("contacts", contacts_total, contacts_total);

    // --- opportunities ----------------------------------------------------
    let stages = list_stages(storage)?;
    let open_stages: Vec<_> = stages
        .iter()
        .filter(|stage| stage.kind == StageKind::Open)
        .cloned()
        .collect();
    let won_stage = stages.iter().find(|stage| stage.kind == StageKind::Won);
    let lost_stage = stages.iter().find(|stage| stage.kind == StageKind::Lost);
    let lost_reasons = list_lost_reasons(storage)?;

    let mut opportunity_ids = Vec::with_capacity(opportunities_total);
    for index in 0..opportunities_total {
        let contact_id = contact_ids[rng.below(contact_ids.len() as u64) as usize].clone();
        let stage = &open_stages[rng.below(open_stages.len().max(1) as u64) as usize];
        let opportunity = create_opportunity(
            storage,
            CreateOpportunityRequest {
                actor: Actor::Import,
                stage_id: Some(stage.id.clone()),
                opportunity: OpportunityPatch {
                    name: format!("{} — {}", pick(JOB_TYPES, &mut rng), pick(CITIES, &mut rng)),
                    contact_id: Some(contact_id),
                    company_id: None,
                    // $800 – $85,000 in cents.
                    value_minor: 80_000 + rng.below(8_420_000) as i64,
                    currency_code: "USD".to_owned(),
                    probability_percent: Some(rng.below(101) as i64),
                    expected_close_date: Some(future_date(&mut rng)),
                    source: Some((*pick(OPPORTUNITY_SOURCES, &mut rng)).to_owned()),
                    source_label: None,
                    notes: Some((*pick(OPPORTUNITY_NOTES, &mut rng)).to_owned()),
                },
            },
        )?;

        // A fifth of the book is already closed — won or lost — so the
        // pipeline board and hand-off surfaces have realistic tails.
        let roll = rng.below(10);
        if roll == 0 {
            if let Some(target) = won_stage {
                crate::application::move_opportunity_stage(
                    storage,
                    MoveOpportunityStageRequest {
                        actor: Actor::Import,
                        opportunity_id: opportunity.id.clone(),
                        to_stage_id: target.id.clone(),
                        lost_reason_id: None,
                        expected_version: opportunity.version,
                    },
                )?;
            }
        } else if roll == 1 {
            if let (Some(target), Some(reason)) = (lost_stage, lost_reasons.first()) {
                crate::application::move_opportunity_stage(
                    storage,
                    MoveOpportunityStageRequest {
                        actor: Actor::Import,
                        opportunity_id: opportunity.id.clone(),
                        to_stage_id: target.id.clone(),
                        lost_reason_id: Some(reason.id.clone()),
                        expected_version: opportunity.version,
                    },
                )?;
            }
        }

        opportunity_ids.push(opportunity.id);
        if index % 500 == 0 {
            progress("opportunities", index, opportunities_total);
        }
    }
    summary.opportunities = opportunity_ids.len();
    progress("opportunities", opportunities_total, opportunities_total);

    // --- activities -------------------------------------------------------
    // The first contact gets a deliberately long timeline (the worst case for
    // get_timeline); the rest are spread over contacts and opportunities.
    let busy_contact = contact_ids.first().cloned();
    for index in 0..activities_total {
        let busy = busy_contact
            .as_ref()
            .filter(|_| index < BUSY_CONTACT_ACTIVITIES);
        let (parent_type, parent_id) = match busy {
            Some(id) => ("contact", id.clone()),
            None => {
                if !opportunity_ids.is_empty() && rng.below(2) == 0 {
                    (
                        "opportunity",
                        opportunity_ids[rng.below(opportunity_ids.len() as u64) as usize].clone(),
                    )
                } else {
                    (
                        "contact",
                        contact_ids[rng.below(contact_ids.len() as u64) as usize].clone(),
                    )
                }
            }
        };
        let kind = *pick(ACTIVITY_KINDS, &mut rng);
        // Notes, site visits, and meetings must carry no direction.
        let direction = if matches!(kind, "note" | "site_visit" | "meeting") {
            "none"
        } else {
            *pick(&["inbound", "outbound", "none"], &mut rng)
        };
        log_activity(
            storage,
            LogActivityRequest {
                actor: Actor::Import,
                parent_type: parent_type.to_owned(),
                parent_id,
                activity: ActivityPatch {
                    kind: kind.to_owned(),
                    direction: Some(direction.to_owned()),
                    occurred_at: Some(past_timestamp(&mut rng)),
                    summary: (*pick(ACTIVITY_SUMMARIES, &mut rng)).to_owned(),
                    body: Some((*pick(ACTIVITY_BODIES, &mut rng)).to_owned()),
                },
            },
        )?;
        if index % 1000 == 0 {
            progress("activities", index, activities_total);
        }
    }
    summary.activities = activities_total;
    progress("activities", activities_total, activities_total);

    // --- tasks ------------------------------------------------------------
    for index in 0..tasks_total {
        let on_opportunity = !opportunity_ids.is_empty() && rng.below(2) == 0;
        let (parent_type, parent_id) = if on_opportunity {
            (
                "opportunity",
                opportunity_ids[rng.below(opportunity_ids.len() as u64) as usize].clone(),
            )
        } else {
            (
                "contact",
                contact_ids[rng.below(contact_ids.len() as u64) as usize].clone(),
            )
        };
        create_task(
            storage,
            CreateTaskRequest {
                actor: Actor::Import,
                task: TaskPatch {
                    title: (*pick(TASK_TITLES, &mut rng)).to_owned(),
                    body: None,
                    parent_type: Some(parent_type.to_owned()),
                    parent_id: Some(parent_id),
                    // A third are already overdue so the attention view has work.
                    due_at: Some(if rng.below(3) == 0 {
                        past_timestamp(&mut rng)
                    } else {
                        future_timestamp(&mut rng)
                    }),
                    remind_at: None,
                    priority: Some((*pick(TASK_PRIORITIES, &mut rng)).to_owned()),
                },
            },
        )?;
        if index % 500 == 0 {
            progress("tasks", index, tasks_total);
        }
    }
    summary.tasks = tasks_total;
    progress("tasks", tasks_total, tasks_total);

    // --- metadata sprinkle ------------------------------------------------
    let crew_options = contact_select_field.options.clone();
    for (index, contact_id) in contact_ids.iter().enumerate() {
        if index % METADATA_EVERY != 0 {
            continue;
        }
        let option = &crew_options[rng.below(crew_options.len().max(1) as u64) as usize];
        set_record_metadata(
            storage,
            SetRecordMetadataRequest {
                actor: Actor::Import,
                entity_type: SavedViewEntityType::Contact,
                record_id: contact_id.clone(),
                expected_version: 1,
                tag_ids: vec![tag_ids[rng.below(tag_ids.len() as u64) as usize].clone()],
                values: vec![
                    CustomFieldValueInput {
                        definition_id: contact_text_field.id.clone(),
                        text_value: Some(format!("#{}", 1000 + rng.below(8999))),
                        number_value: None,
                        date_value: None,
                        option_id: None,
                    },
                    CustomFieldValueInput {
                        definition_id: contact_select_field.id.clone(),
                        text_value: None,
                        number_value: None,
                        date_value: None,
                        option_id: Some(option.id.clone()),
                    },
                ],
            },
        )?;
        summary.records_with_metadata += 1;
    }
    for (index, opportunity_id) in opportunity_ids.iter().enumerate() {
        if index % METADATA_EVERY != 0 {
            continue;
        }
        // Closed opportunities were moved a stage, so their version moved too.
        let version = crate::application::get_opportunity(storage, opportunity_id)?
            .opportunity
            .version;
        set_record_metadata(
            storage,
            SetRecordMetadataRequest {
                actor: Actor::Import,
                entity_type: SavedViewEntityType::Opportunity,
                record_id: opportunity_id.clone(),
                expected_version: version,
                tag_ids: vec![tag_ids[rng.below(tag_ids.len() as u64) as usize].clone()],
                values: vec![CustomFieldValueInput {
                    definition_id: opportunity_number_field.id.clone(),
                    text_value: None,
                    number_value: Some(60.0 + rng.below(1200) as f64),
                    date_value: None,
                    option_id: None,
                }],
            },
        )?;
        summary.records_with_metadata += 1;
    }
    progress(
        "metadata",
        summary.records_with_metadata,
        summary.records_with_metadata,
    );

    Ok(summary)
}

fn scaled(contacts: usize, ratio: f64) -> usize {
    (contacts as f64 * ratio).round() as usize
}

// ---------------------------------------------------------------------------
// Deterministic RNG — xorshift64*, small enough not to justify a dependency.
// ---------------------------------------------------------------------------

/// Seeded pseudo-random source. Not cryptographic; it only has to be stable.
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        // A zero state would stick at zero forever.
        Self {
            state: if seed == 0 {
                0x9E37_79B9_7F4A_7C15
            } else {
                seed
            },
        }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform-enough value in `0..bound` (bound must be non-zero).
    fn below(&mut self, bound: u64) -> u64 {
        if bound == 0 {
            return 0;
        }
        self.next_u64() % bound
    }
}

fn pick<'a, T>(values: &'a [T], rng: &mut Rng) -> &'a T {
    &values[rng.below(values.len() as u64) as usize]
}

fn phone_number(rng: &mut Rng) -> String {
    format!("(407) {:03}-{:04}", 200 + rng.below(700), rng.below(10_000))
}

fn street_address(rng: &mut Rng) -> String {
    format!("{} {}", 100 + rng.below(9_900), pick(STREETS, rng))
}

fn postal_code(rng: &mut Rng) -> String {
    format!("3{:04}", 2000 + rng.below(2999))
}

fn company_name(rng: &mut Rng, index: usize) -> String {
    format!(
        "{} {} {}",
        pick(COMPANY_PREFIXES, rng),
        pick(COMPANY_TRADES, rng),
        // Keeps names unique so search hits are countable.
        index
    )
}

/// A timestamp up to two years in the past.
fn past_timestamp(rng: &mut Rng) -> String {
    let minutes = rng.below(2 * 365 * 24 * 60) as i64;
    (Utc::now() - Duration::minutes(minutes)).to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// A timestamp up to 120 days out.
fn future_timestamp(rng: &mut Rng) -> String {
    let minutes = rng.below(120 * 24 * 60) as i64;
    (Utc::now() + Duration::minutes(minutes)).to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// A calendar date up to 180 days out (expected close dates).
fn future_date(rng: &mut Rng) -> String {
    let days = rng.below(180) as i64;
    (Utc::now() + Duration::days(days))
        .format("%Y-%m-%d")
        .to_string()
}

// ---------------------------------------------------------------------------
// Contractor-flavoured word lists
// ---------------------------------------------------------------------------

const FIRST_NAMES: &[&str] = &[
    "Dale", "Maria", "Travis", "Wanda", "Hector", "Bobby", "Luis", "Sandra", "Curtis", "Ronnie",
    "Angela", "Duane", "Rosa", "Kenny", "Lorraine", "Marcus", "Pete", "Yolanda", "Wes", "Ginny",
    "Otis", "Darlene", "Ramon", "Tammy", "Clyde", "Brenda", "Vince", "Shonda", "Earl", "Nadine",
];

const LAST_NAMES: &[&str] = &[
    "Whitfield",
    "Alvarez",
    "Boone",
    "Castillo",
    "Doyle",
    "Ferris",
    "Gault",
    "Hobbs",
    "Ingram",
    "Jessup",
    "Kirkland",
    "Lassiter",
    "Maddox",
    "Nunez",
    "Ortega",
    "Pruitt",
    "Quesada",
    "Rankin",
    "Stroud",
    "Tilley",
    "Underwood",
    "Vance",
    "Whitaker",
    "Yarborough",
    "Zamora",
];

const CITIES: &[&str] = &[
    "Orlando",
    "Kissimmee",
    "Sanford",
    "Winter Garden",
    "Ocoee",
    "Apopka",
    "Clermont",
    "Deltona",
    "Lake Mary",
    "St. Cloud",
    "Oviedo",
    "Leesburg",
];

const STREETS: &[&str] = &[
    "Orange Blossom Trl",
    "Palmetto Ave",
    "Sand Lake Rd",
    "Old Winter Garden Rd",
    "Pine Hills Rd",
    "Colonial Dr",
    "Silver Star Rd",
    "Goldenrod Rd",
    "Hiawassee Rd",
    "Curry Ford Rd",
];

const COMPANY_PREFIXES: &[&str] = &[
    "Sunstate",
    "Lakeview",
    "Central Florida",
    "Ridgeline",
    "Cypress",
    "Blue Heron",
    "Anchor",
    "First Coast",
    "Palmetto",
    "Reliant",
];

const COMPANY_TRADES: &[&str] = &[
    "Fence & Gate",
    "Site Work",
    "Grading",
    "Builders",
    "Property Group",
    "Concrete",
    "Landscape",
    "Homes",
    "Development",
    "Supply",
];

const COMPANY_KINDS: &[&str] = &["client", "lead", "sub", "vendor", "supplier", "other"];

const CONTACT_KINDS: &[&str] = &["client", "lead", "lead", "sub", "vendor", "other"];

const CONTACT_ROLES: &[&str] = &["owner", "estimator", "site_contact", "office", "other"];

const PROPERTY_TYPES: &[&str] = &[
    "single family",
    "townhome",
    "acreage",
    "commercial",
    "HOA common area",
    "industrial yard",
];

const CONTACT_NOTES: &[&str] = &[
    "Dog in the back yard — call before the crew rolls up.",
    "Gate on the south side; HOA approval already on file.",
    "Prefers a text the morning of the site visit.",
    "Repeat customer from the 2023 back-yard job.",
    "Wants the survey pins located before any post goes in.",
    "Corner lot, easement on the east line.",
];

const COMPANY_NOTES: &[&str] = &[
    "Pays on net-30, PO required on every invoice.",
    "Superintendent walks the site Friday mornings.",
    "Certificate of insurance is on file through year end.",
    "Uses their own survey crew for layout.",
];

const JOB_TYPES: &[&str] = &[
    "6' vinyl privacy",
    "4' aluminum pool fence",
    "Chain link back yard",
    "Board-on-board replacement",
    "Ranch rail pasture",
    "Cantilever slide gate",
    "Dumpster enclosure",
    "Construction temp fence",
];

const OPPORTUNITY_SOURCES: &[&str] = &["referral", "repeat_client", "website", "sign", "other"];

const OPPORTUNITY_NOTES: &[&str] = &[
    "Needs the HOA color match before we order material.",
    "Access is tight — no room for the auger truck on the north side.",
    "Customer is getting two other quotes; decision after the holiday.",
    "Permit will be pulled by the GC, we install after inspection.",
];

const ACTIVITY_KINDS: &[&str] = &["call", "email", "text", "site_visit", "meeting", "note"];

const ACTIVITY_SUMMARIES: &[&str] = &[
    "Left a voicemail about the site visit",
    "Emailed the revised quote",
    "Texted the crew arrival window",
    "Walked the property and measured the run",
    "Met at the job trailer with the superintendent",
    "Logged the deposit check",
    "Confirmed gate swing direction",
    "Called about the material back-order",
];

const ACTIVITY_BODIES: &[&str] = &[
    "Customer asked about upgrading to a heavier gate frame.",
    "Utility locate is scheduled; nothing gets dug until it clears.",
    "Grade drops about two feet across the back run — stepping the panels.",
    "Left the takeoff sheet in the truck; sending a photo tonight.",
];

const TASK_TITLES: &[&str] = &[
    "Follow up on the quote",
    "Call for the utility locate",
    "Send the HOA the color sample",
    "Schedule the site visit",
    "Collect the deposit",
    "Order gate hardware",
    "Confirm the permit number",
];

const TASK_PRIORITIES: &[&str] = &["low", "normal", "normal", "high"];

const TAG_LABELS: &[&str] = &[
    "Repeat client",
    "HOA",
    "Commercial",
    "Needs permit",
    "Referral",
    "Warranty",
];

const CREW_NAMES: &[&str] = &["Crew A", "Crew B", "Crew C", "Subcontracted"];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::{list_contacts, list_opportunities, list_tags, search_records};

    /// Small on purpose — this runs in CI, the 10k run does not.
    #[test]
    fn seeds_a_small_consistent_database() {
        let mut storage = Storage::open_in_memory().expect("open");
        let summary = seed_database(
            &mut storage,
            &SeedOptions {
                contacts: 200,
                seed: 7,
            },
            |_, _, _| {},
        )
        .expect("seed");

        assert_eq!(summary.contacts, 200);
        assert_eq!(summary.companies, 40);
        assert_eq!(summary.opportunities, 100);
        assert_eq!(summary.activities, 600);
        assert_eq!(summary.tasks, 100);
        assert_eq!(list_contacts(&storage, false).expect("list").len(), 200);
        assert_eq!(
            list_opportunities(&storage, false).expect("list").len(),
            100
        );
        assert_eq!(list_tags(&storage, false).expect("tags").len(), 6);

        // The seam kept the FTS projection in step with the records.
        let hits = search_records(
            &storage,
            "vinyl".into(),
            Some(vec!["opportunity".into()]),
            None,
        )
        .expect("search");
        assert!(
            !hits.is_empty(),
            "seeded opportunities should be searchable"
        );
    }

    #[test]
    fn same_seed_produces_the_same_names() {
        let names = |seed: u64| {
            let mut storage = Storage::open_in_memory().expect("open");
            seed_database(
                &mut storage,
                &SeedOptions { contacts: 20, seed },
                |_, _, _| {},
            )
            .expect("seed");
            list_contacts(&storage, false)
                .expect("list")
                .into_iter()
                .map(|row| row.contact.display_name)
                .collect::<Vec<_>>()
        };
        assert_eq!(names(11), names(11));
        assert_ne!(names(11), names(12));
    }
}
