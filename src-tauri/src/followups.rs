//! History summaries and follow-up drafting.
//!
//! Three things live here, in the order a contractor meets them:
//!
//! * **Templates.** A small built-in set of follow-up wordings stored as
//!   versioned JSON in `app_settings` (the ai-settings pattern). They are
//!   plain text and work with the assistant switched off — the model only ever
//!   personalizes one, it never invents the set.
//! * **`summarize_history`.** A bounded projection of one record's timeline,
//!   sent through the provider seam for a short summary plus suggested next
//!   actions. Explanation only: no proposal, no writes.
//! * **`propose_followup`.** Picks a template, drafts the wording (through the
//!   provider when it is on, verbatim when it is off), and returns a proposal
//!   for a follow-up task. Applying it is the user's separate, explicit act.
//!
//! Two invariants shape the code, both inherited from `ai.rs`:
//!
//! * **Bounded projection.** Only the target record's identity and its own
//!   recent activity entries leave the machine — entry count capped, bodies
//!   truncated, no other record's data, no attachments, no credentials.
//! * **Mutex rule.** Nothing here calls the network with the storage lock
//!   held. Summaries use a plan/run split; follow-up drafting is
//!   lock → project → unlock → call → lock.

use std::sync::{Mutex, MutexGuard};

use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::ai::{
    self, CompletionProvider, ContextPreview, CredentialStore, OpenAiCompatibleProvider,
    ProviderRequest, RecordRef,
};
use crate::application::{self, immediate, log_command, TaskPatch};
use crate::domain::{Activity, Actor};
use crate::error::ApplicationError;
use crate::proposals::{self, Proposal, ProposalStore};
use crate::storage::Storage;

/// app_settings key holding the versioned template set.
const TEMPLATES_KEY: &str = "followups.templates";

/// Schema version of the stored template blob.
pub const FOLLOWUP_TEMPLATES_VERSION: u32 = 1;

/// Caps on the stored set, so a bad write cannot bloat settings or a prompt.
const MAX_TEMPLATES: usize = 20;
const MAX_TEMPLATE_ID_CHARS: usize = 60;
const MAX_TEMPLATE_NAME_CHARS: usize = 80;
const MAX_TEMPLATE_BODY_CHARS: usize = 2000;

/// Feature names carried on the requests this module builds.
pub const SUMMARIZE_PURPOSE: &str = "summarize_history";
pub const FOLLOWUP_PURPOSE: &str = "propose_followup";

/// How far back a summary looks when the caller does not say.
pub const DEFAULT_SUMMARY_WINDOW_DAYS: i64 = 90;
const MAX_SUMMARY_WINDOW_DAYS: i64 = 3650;

/// Projection bounds: how many timeline entries are included and how much of
/// each one. A long note is a summary's job to compress, not the prompt's.
const MAX_TIMELINE_ENTRIES: usize = 25;
const MAX_ENTRY_BODY_CHARS: usize = 200;
const MAX_ENTRY_SUMMARY_CHARS: usize = 120;

/// Output caps — a recap and a short drafted message, not a report.
const MAX_SUMMARY_OUTPUT_TOKENS: u32 = 600;
const MAX_DRAFT_OUTPUT_TOKENS: u32 = 500;

/// Most next actions ever returned, and the longest each one may be.
const MAX_SUGGESTED_ACTIONS: usize = 5;
const MAX_ACTION_CHARS: usize = 200;

/// Longest objective accepted, and longest drafted follow-up kept.
const MAX_OBJECTIVE_CHARS: usize = 500;
const MAX_DRAFT_CHARS: usize = 4000;

const SUMMARY_SYSTEM_TEXT: &str = "You help a contractor keep track of a job or a client. Using \
only the history you are given, write a short recap in three or four sentences, then list the \
next actions. Never invent names, dates, prices, or promises that are not in the history.";

const SUMMARY_USER_TEXT: &str = "Recap this record's history, then say what to do next.\n\nAnswer \
in this shape:\nSummary: <a few sentences>\nNext actions:\n- <one action per line>";

const DRAFT_SYSTEM_TEXT: &str = "You help a contractor write a short follow-up message. Start \
from the template wording you are given and adjust it to fit the history. Keep it plain, warm, \
and under 150 words. Reply with the message text only — no subject line, no notes, no \
explanation. Never invent prices, dates, or commitments that are not in the history.";

// ---------------------------------------------------------------------------
// Templates
// ---------------------------------------------------------------------------

/// One reusable follow-up wording. Plain text on purpose: it is usable exactly
/// as written when the assistant is off.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FollowupTemplate {
    pub id: String,
    pub name: String,
    pub body: String,
}

/// The stored set, versioned like every other settings blob.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FollowupTemplates {
    pub version: u32,
    pub templates: Vec<FollowupTemplate>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetFollowupTemplatesRequest {
    #[serde(default)]
    pub actor: Actor,
    pub templates: Vec<FollowupTemplate>,
}

/// The built-in set: the three follow-ups a contractor writes most often.
pub fn default_templates() -> Vec<FollowupTemplate> {
    [
        (
            "call_followup",
            "Call follow-up",
            "Thanks for taking the time on the phone. Here's a quick recap of what we \
             talked about and what happens next on my end. If I missed anything or you \
             think of something else, just let me know and I'll work it in.",
        ),
        (
            "proposal_chaser",
            "Proposal follow-up",
            "I wanted to check in on the proposal I sent over. Happy to walk through the \
             pricing line by line or adjust the scope if something needs to change. When \
             you're ready to move forward, let me know and I'll get you on the schedule.",
        ),
        (
            "site_visit_note",
            "Post-site-visit note",
            "Thanks for having me out to the property. I got what I needed from the walk \
             and I'm putting the numbers together based on what we discussed on site. \
             I'll have it back to you shortly — call me in the meantime with any questions.",
        ),
    ]
    .into_iter()
    .map(|(id, name, body)| FollowupTemplate {
        id: id.to_owned(),
        name: name.to_owned(),
        body: body.to_owned(),
    })
    .collect()
}

/// Read the template set, falling back to the built-ins when the user has
/// never edited them.
pub fn get_followup_templates(storage: &Storage) -> Result<FollowupTemplates, ApplicationError> {
    // `.optional()` and not `.ok()`: "no row" means the built-ins, but a real
    // database failure must surface instead of quietly looking unconfigured.
    let value: Option<String> = storage
        .connection()
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            [TEMPLATES_KEY],
            |row| row.get(0),
        )
        .optional()?;
    let Some(value) = value else {
        return Ok(FollowupTemplates {
            version: FOLLOWUP_TEMPLATES_VERSION,
            templates: default_templates(),
        });
    };
    let stored = serde_json::from_str::<FollowupTemplates>(&value).map_err(|error| {
        ApplicationError::InvalidStoredData(format!(
            "app_settings {TEMPLATES_KEY} holds invalid JSON: {error}"
        ))
    })?;
    if stored.version != FOLLOWUP_TEMPLATES_VERSION {
        return Err(ApplicationError::InvalidStoredData(format!(
            "app_settings {TEMPLATES_KEY} has unsupported version {}",
            stored.version
        )));
    }
    Ok(stored)
}

/// Replace the template set. An empty list restores the built-ins, so the
/// editor can never leave a contractor with nothing to send.
pub fn set_followup_templates(
    storage: &mut Storage,
    request: SetFollowupTemplatesRequest,
) -> Result<FollowupTemplates, ApplicationError> {
    let templates = validated_templates(request.templates)?;
    let stored = FollowupTemplates {
        version: FOLLOWUP_TEMPLATES_VERSION,
        templates,
    };
    let value = serde_json::to_string(&stored).map_err(|error| {
        ApplicationError::InvalidStoredData(format!(
            "follow-up templates could not be encoded: {error}"
        ))
    })?;

    let transaction = immediate(storage)?;
    transaction.execute(
        "INSERT INTO app_settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![TEMPLATES_KEY, value],
    )?;
    log_command(
        &transaction,
        request.actor,
        "settings",
        "followup_templates",
        "updated the follow-up templates",
    )?;
    transaction.commit()?;
    Ok(stored)
}

fn validated_templates(
    templates: Vec<FollowupTemplate>,
) -> Result<Vec<FollowupTemplate>, ApplicationError> {
    if templates.is_empty() {
        return Ok(default_templates());
    }
    if templates.len() > MAX_TEMPLATES {
        return Err(ApplicationError::InvalidInput {
            field: "templates".into(),
            message: format!("keep {MAX_TEMPLATES} templates or fewer"),
        });
    }
    let mut checked: Vec<FollowupTemplate> = Vec::with_capacity(templates.len());
    for template in templates {
        let name = checked_text("name", template.name, MAX_TEMPLATE_NAME_CHARS)?;
        let body = checked_text("body", template.body, MAX_TEMPLATE_BODY_CHARS)?;
        // A blank id is filled from the name so the editor never has to invent
        // one; ids only have to be unique and short.
        let id = match template.id.trim() {
            "" => slug(&name),
            value => checked_text("id", value.to_owned(), MAX_TEMPLATE_ID_CHARS)?,
        };
        if checked.iter().any(|existing| existing.id == id) {
            return Err(ApplicationError::InvalidInput {
                field: "id".into(),
                message: format!("template \"{id}\" is listed twice"),
            });
        }
        checked.push(FollowupTemplate { id, name, body });
    }
    Ok(checked)
}

fn checked_text(field: &str, value: String, max: usize) -> Result<String, ApplicationError> {
    let value = value.trim().to_owned();
    if value.is_empty() {
        return Err(ApplicationError::InvalidInput {
            field: field.into(),
            message: "is required".into(),
        });
    }
    if value.chars().count() > max {
        return Err(ApplicationError::InvalidInput {
            field: field.into(),
            message: format!("must be {max} characters or fewer"),
        });
    }
    Ok(value)
}

/// Lowercase, underscore-joined id derived from a template name.
fn slug(name: &str) -> String {
    let slug = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    let slug = slug
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    slug.chars().take(MAX_TEMPLATE_ID_CHARS).collect()
}

// ---------------------------------------------------------------------------
// Shared projection of one record's recent history
// ---------------------------------------------------------------------------

/// The bounded facts a provider call may see: who the record is, and its own
/// recent activity. Never another record's data, and never a file's contents.
struct HistoryProjection {
    record: RecordRef,
    text: String,
    entry_count: usize,
}

/// Read the record's label and its capped, truncated timeline. Runs under the
/// storage lock; sends nothing.
fn project_history(
    storage: &Storage,
    parent_type: &str,
    parent_id: &str,
    window_days: i64,
    now: DateTime<Utc>,
) -> Result<HistoryProjection, ApplicationError> {
    let record = parent_record_ref(storage, parent_type, parent_id)?;
    // Related-record activity is deliberately excluded: a summary of this
    // record must not carry another record's notes off the machine.
    let entries = application::get_timeline(storage, parent_type, parent_id, false)?;
    let cutoff = now - Duration::days(window_days);

    let within_window = entries
        .iter()
        .filter(|entry| occurred_at(entry).is_none_or(|occurred| occurred >= cutoff))
        .collect::<Vec<_>>();
    let included = within_window.iter().take(MAX_TIMELINE_ENTRIES);

    let mut lines = vec![
        format!(
            "Record: {} ({} {})",
            record.label, record.entity_type, record.entity_id
        ),
        format!("Today: {}", day(now)),
        format!(
            "Window: the last {window_days} day(s), since {}",
            day(cutoff)
        ),
    ];
    if within_window.is_empty() {
        lines.push("Activity: none logged in this window.".into());
    } else {
        lines.push("Activity (newest first):".into());
        for entry in included {
            lines.push(entry_line(entry));
        }
        if within_window.len() > MAX_TIMELINE_ENTRIES {
            lines.push(format!(
                "({} older entr{} in this window were left out.)",
                within_window.len() - MAX_TIMELINE_ENTRIES,
                if within_window.len() - MAX_TIMELINE_ENTRIES == 1 {
                    "y"
                } else {
                    "ies"
                }
            ));
        }
    }

    Ok(HistoryProjection {
        record,
        text: lines.join("\n"),
        entry_count: within_window.len().min(MAX_TIMELINE_ENTRIES),
    })
}

/// One timeline entry as a single bounded line.
fn entry_line(entry: &Activity) -> String {
    let when = occurred_at(entry)
        .map(day)
        .unwrap_or_else(|| entry.occurred_at.clone());
    let direction = match entry.direction.as_database_value() {
        "none" => String::new(),
        value => format!(" {value}"),
    };
    let mut line = format!(
        "- {when} {}{direction}: {}",
        entry.kind.as_database_value(),
        truncate(&entry.summary, MAX_ENTRY_SUMMARY_CHARS)
    );
    if let Some(body) = entry.body.as_ref().filter(|body| !body.trim().is_empty()) {
        line.push_str(&format!(" — {}", truncate(body, MAX_ENTRY_BODY_CHARS)));
    }
    line
}

/// The one record whose data is in the projection — the disclosure list.
fn parent_record_ref(
    storage: &Storage,
    parent_type: &str,
    parent_id: &str,
) -> Result<RecordRef, ApplicationError> {
    let label = match parent_type {
        "contact" => application::get_contact(storage, parent_id)?.display_name,
        "company" => application::get_company(storage, parent_id)?.name,
        "opportunity" => {
            application::get_opportunity(storage, parent_id)?
                .opportunity
                .name
        }
        other => {
            return Err(ApplicationError::InvalidInput {
                field: "parentType".into(),
                message: format!(
                    "unknown parent type \"{other}\"; expected one of contact, company, opportunity"
                ),
            })
        }
    };
    Ok(RecordRef {
        entity_type: parent_type.to_owned(),
        entity_id: parent_id.to_owned(),
        label,
    })
}

fn checked_window(window: Option<i64>) -> Result<i64, ApplicationError> {
    let window = window.unwrap_or(DEFAULT_SUMMARY_WINDOW_DAYS);
    if !(1..=MAX_SUMMARY_WINDOW_DAYS).contains(&window) {
        return Err(ApplicationError::InvalidInput {
            field: "window".into(),
            message: format!("must be between 1 and {MAX_SUMMARY_WINDOW_DAYS} days"),
        });
    }
    Ok(window)
}

fn occurred_at(entry: &Activity) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&entry.occurred_at)
        .ok()
        .map(|parsed| parsed.with_timezone(&Utc))
}

fn day(timestamp: DateTime<Utc>) -> String {
    timestamp.format("%Y-%m-%d").to_string()
}

fn truncate(value: &str, max: usize) -> String {
    let value = value.trim();
    if value.chars().count() <= max {
        return value.to_owned();
    }
    value.chars().take(max).collect::<String>() + "…"
}

// ---------------------------------------------------------------------------
// summarize_history (explanation only)
// ---------------------------------------------------------------------------

/// A recap plus suggested next actions, with the provenance the UI shows.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistorySummary {
    pub parent_type: String,
    pub parent_id: String,
    /// Host (with port) the request went to, for the disclosure line.
    pub endpoint_host: String,
    /// True when that host is on this machine — no data left the device.
    pub local: bool,
    pub model: String,
    pub summary: String,
    /// Parsed suggestions; empty when the model answered in prose only.
    pub suggested_next_actions: Vec<String>,
    pub included_record_refs: Vec<RecordRef>,
}

/// A prepared summary call: everything read from storage, nothing sent.
pub struct SummaryPlan {
    parent_type: String,
    parent_id: String,
    endpoint_host: String,
    local: bool,
    entry_count: usize,
    provider: OpenAiCompatibleProvider,
    request: ProviderRequest,
}

/// Debug by hand: the provider holds the API key and must never be printed.
impl std::fmt::Debug for SummaryPlan {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SummaryPlan")
            .field("parent_type", &self.parent_type)
            .field("parent_id", &self.parent_id)
            .field("endpoint_host", &self.endpoint_host)
            .field("local", &self.local)
            .field("request", &self.request)
            .finish_non_exhaustive()
    }
}

impl SummaryPlan {
    /// The exact request that would be sent — inspectable before it goes out.
    pub fn request(&self) -> &ProviderRequest {
        &self.request
    }

    /// How many timeline entries the projection carries.
    pub fn entry_count(&self) -> usize {
        self.entry_count
    }

    /// Send the planned call. Only after the storage lock has been released.
    pub fn run(&self) -> Result<HistorySummary, ApplicationError> {
        self.run_with(&self.provider)
    }

    /// Same, against any provider — the seam tests use a canned one.
    pub fn run_with(
        &self,
        provider: &dyn CompletionProvider,
    ) -> Result<HistorySummary, ApplicationError> {
        let completion = provider.complete(&self.request)?;
        let (summary, suggested_next_actions) = parse_summary(&completion.text);
        Ok(HistorySummary {
            parent_type: self.parent_type.clone(),
            parent_id: self.parent_id.clone(),
            endpoint_host: self.endpoint_host.clone(),
            local: self.local,
            model: completion.model,
            summary,
            suggested_next_actions,
            included_record_refs: completion.included_record_refs,
        })
    }
}

/// Build the bounded projection for one record's history. A switched-off or
/// unconfigured assistant is `provider_unavailable` and reads no credentials.
pub fn plan_history_summary(
    storage: &Storage,
    credentials: &dyn CredentialStore,
    parent_type: &str,
    parent_id: &str,
    window: Option<i64>,
) -> Result<SummaryPlan, ApplicationError> {
    // Settings first: an assistant that is off must not read the credential
    // store, and must not send anything anywhere.
    let Some(provider) = ai::configured_provider(storage, credentials)? else {
        return Err(ApplicationError::ProviderUnavailable {
            reason: "The AI assistant is off. Turn it on in Settings to summarize history.".into(),
        });
    };
    let window = checked_window(window)?;
    let projection = project_history(storage, parent_type, parent_id, window, Utc::now())?;

    Ok(SummaryPlan {
        parent_type: parent_type.to_owned(),
        parent_id: parent_id.to_owned(),
        endpoint_host: ai::endpoint_host(provider.base_url()),
        local: ai::is_local_endpoint(provider.base_url()),
        entry_count: projection.entry_count,
        provider,
        request: ProviderRequest {
            purpose: SUMMARIZE_PURPOSE.into(),
            system_text: SUMMARY_SYSTEM_TEXT.into(),
            user_text: SUMMARY_USER_TEXT.into(),
            context_text: Some(projection.text),
            included_record_refs: vec![projection.record],
            max_output_tokens: Some(MAX_SUMMARY_OUTPUT_TOKENS),
            timeout_seconds: None,
        },
    })
}

/// The same bounded history projection `summarize_history` would send, built
/// without a provider and without reading credentials — the agent interface's
/// "what would be sent" surface.
pub fn preview_history_context(
    storage: &Storage,
    parent_type: &str,
    parent_id: &str,
    window: Option<i64>,
) -> Result<ContextPreview, ApplicationError> {
    let window = checked_window(window)?;
    let projection = project_history(storage, parent_type, parent_id, window, Utc::now())?;
    Ok(ContextPreview {
        purpose: SUMMARIZE_PURPOSE.into(),
        context_text: projection.text,
        included_record_refs: vec![projection.record],
    })
}

/// The same bounded history projection `propose_followup` would send. Drafting
/// always looks back `DEFAULT_SUMMARY_WINDOW_DAYS`, so this takes no window —
/// the preview would otherwise be able to disagree with the real call.
pub fn preview_followup_context(
    storage: &Storage,
    parent_type: &str,
    parent_id: &str,
) -> Result<ContextPreview, ApplicationError> {
    let projection = project_history(
        storage,
        parent_type,
        parent_id,
        DEFAULT_SUMMARY_WINDOW_DAYS,
        Utc::now(),
    )?;
    Ok(ContextPreview {
        purpose: FOLLOWUP_PURPOSE.into(),
        context_text: projection.text,
        included_record_refs: vec![projection.record],
    })
}

/// Split a model answer into the recap and the suggested actions. Defensive:
/// an answer with no "next actions" heading is all recap, and bullet markers,
/// numbering, and blank lines are all tolerated.
fn parse_summary(text: &str) -> (String, Vec<String>) {
    let lines = text.lines().collect::<Vec<_>>();
    let heading = lines.iter().position(|line| {
        let line = line.trim().trim_start_matches(['-', '*', '•', '#', ' ']);
        let lowered = line.to_lowercase();
        lowered.starts_with("next action") || lowered.starts_with("next step")
    });

    let (summary_lines, action_lines) = match heading {
        Some(index) => (&lines[..index], &lines[index + 1..]),
        None => (&lines[..], &lines[lines.len()..]),
    };

    let summary = summary_lines
        .iter()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .map(|line| {
            line.strip_prefix("Summary:")
                .or_else(|| line.strip_prefix("summary:"))
                .unwrap_or(line)
                .trim()
        })
        .collect::<Vec<_>>()
        .join(" ");

    let actions = action_lines
        .iter()
        .map(|line| {
            line.trim()
                .trim_start_matches(['-', '*', '•', ' '])
                .trim_start_matches(|character: char| character.is_ascii_digit())
                .trim_start_matches(['.', ')', ' '])
                .trim()
        })
        .filter(|line| !line.is_empty())
        .take(MAX_SUGGESTED_ACTIONS)
        .map(|line| truncate(line, MAX_ACTION_CHARS))
        .collect::<Vec<_>>();

    (summary.trim().to_owned(), actions)
}

// ---------------------------------------------------------------------------
// propose_followup (drafted wording + a task proposal)
// ---------------------------------------------------------------------------

/// Drafted follow-up wording plus the proposal that would file it as a task.
/// Nothing is written until the user applies the proposal.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FollowupDraft {
    pub parent_type: String,
    pub parent_id: String,
    pub template_id: String,
    pub template_name: String,
    pub draft_text: String,
    /// False when the assistant is off: the template body is used verbatim.
    pub used_provider: bool,
    /// Where the request went; absent in template-only mode.
    pub endpoint_host: Option<String>,
    pub local: bool,
    pub model: Option<String>,
    /// Records whose data was in the call; empty in template-only mode.
    pub included_record_refs: Vec<RecordRef>,
    pub warnings: Vec<String>,
    pub proposal: Proposal,
}

fn lock_storage(storage: &Mutex<Storage>) -> MutexGuard<'_, Storage> {
    storage.lock().expect("storage mutex poisoned")
}

/// Draft a follow-up for one record. With the assistant on, the provider
/// personalizes the chosen template against the record's recent history; with
/// it off, the template is returned verbatim and nothing is sent anywhere.
/// Either way a follow-up task proposal comes back for the user to review.
/// `template_id` picks a template outright; absent, one is chosen from the
/// objective (see `pick_template`).
pub fn propose_followup(
    storage: &Mutex<Storage>,
    credentials: &dyn CredentialStore,
    store: &ProposalStore,
    parent_type: &str,
    parent_id: &str,
    objective: Option<&str>,
    template_id: Option<&str>,
) -> Result<FollowupDraft, ApplicationError> {
    let provider = {
        let guard = lock_storage(storage);
        ai::configured_provider(&guard, credentials)?
    };
    propose_followup_with(
        storage,
        provider
            .as_ref()
            .map(|provider| provider as &dyn CompletionProvider),
        store,
        parent_type,
        parent_id,
        objective,
        template_id,
    )
}

/// Same flow against an explicit provider (`None` = template-only) — what the
/// seam tests drive.
#[allow(clippy::too_many_arguments)]
pub fn propose_followup_with(
    storage: &Mutex<Storage>,
    provider: Option<&dyn CompletionProvider>,
    store: &ProposalStore,
    parent_type: &str,
    parent_id: &str,
    objective: Option<&str>,
    template_id: Option<&str>,
) -> Result<FollowupDraft, ApplicationError> {
    let objective = checked_objective(objective)?;

    // Everything the call needs is read under the lock, which is then released
    // for the call itself.
    let (templates, projection) = {
        let guard = lock_storage(storage);
        let templates = get_followup_templates(&guard)?.templates;
        let projection = project_history(
            &guard,
            parent_type,
            parent_id,
            DEFAULT_SUMMARY_WINDOW_DAYS,
            Utc::now(),
        )?;
        (templates, projection)
    };
    let template = pick_template(&templates, objective.as_deref(), template_id)?;
    let mut warnings = Vec::new();

    let (draft_text, used_provider, model, included_record_refs) = match provider {
        None => (template.body.clone(), false, None, Vec::new()),
        Some(provider) => {
            let completion = provider.complete(&ProviderRequest {
                purpose: FOLLOWUP_PURPOSE.into(),
                system_text: DRAFT_SYSTEM_TEXT.into(),
                user_text: draft_prompt(template, objective.as_deref()),
                context_text: Some(projection.text.clone()),
                included_record_refs: vec![projection.record.clone()],
                max_output_tokens: Some(MAX_DRAFT_OUTPUT_TOKENS),
                timeout_seconds: None,
            })?;
            let text = completion.text.trim().to_owned();
            if text.is_empty() {
                // An empty answer is not an error — the template still works.
                warnings.push(
                    "The assistant sent nothing back, so the template is used as written.".into(),
                );
                (
                    template.body.clone(),
                    false,
                    Some(completion.model),
                    completion.included_record_refs,
                )
            } else {
                (
                    truncate(&text, MAX_DRAFT_CHARS),
                    true,
                    Some(completion.model),
                    completion.included_record_refs,
                )
            }
        }
    };

    let task = TaskPatch {
        title: truncate(
            &objective
                .clone()
                .unwrap_or_else(|| format!("Follow up: {}", projection.record.label)),
            200,
        ),
        body: Some(draft_text.clone()),
        parent_type: Some(parent_type.to_owned()),
        parent_id: Some(parent_id.to_owned()),
        // No due date is invented here — the user sets one when they want it.
        due_at: None,
        remind_at: None,
        priority: None,
    };
    let proposal = proposals::followup_task_proposal(
        store,
        task,
        format!("Follow up with {}", projection.record.label),
        warnings.clone(),
    )?;

    // Disclosure line comes from the provider that was actually used.
    let (endpoint_host, local) = match provider.and_then(CompletionProvider::endpoint) {
        Some(base_url) => (
            Some(ai::endpoint_host(base_url)),
            ai::is_local_endpoint(base_url),
        ),
        None => (None, false),
    };
    Ok(FollowupDraft {
        parent_type: parent_type.to_owned(),
        parent_id: parent_id.to_owned(),
        template_id: template.id.clone(),
        template_name: template.name.clone(),
        draft_text,
        used_provider,
        endpoint_host,
        local,
        model,
        included_record_refs,
        warnings,
        proposal,
    })
}

fn checked_objective(objective: Option<&str>) -> Result<Option<String>, ApplicationError> {
    let Some(objective) = objective.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if objective.chars().count() > MAX_OBJECTIVE_CHARS {
        return Err(ApplicationError::InvalidInput {
            field: "objective".into(),
            message: format!("must be {MAX_OBJECTIVE_CHARS} characters or fewer"),
        });
    }
    Ok(Some(objective.to_owned()))
}

/// Choose a template deterministically: the best word overlap between the
/// objective and a template's name or id, and the first template when there is
/// no objective or nothing matches. Templates are never empty (an empty set
/// restores the built-ins), so this always has something to return.
fn pick_template<'a>(
    templates: &'a [FollowupTemplate],
    objective: Option<&str>,
    template_id: Option<&str>,
) -> Result<&'a FollowupTemplate, ApplicationError> {
    let fallback = templates.first().expect("template set is never empty");
    // An explicit choice wins; an id that is not on file is the caller's error
    // rather than a silent substitution.
    if let Some(template_id) = template_id.map(str::trim).filter(|id| !id.is_empty()) {
        return templates
            .iter()
            .find(|template| template.id == template_id)
            .ok_or_else(|| ApplicationError::NotFound {
                resource: "follow-up template",
                id: template_id.to_owned(),
            });
    }
    let Some(objective) = objective else {
        return Ok(fallback);
    };
    let words = objective
        .to_lowercase()
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| word.len() > 2)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    Ok(templates
        .iter()
        .map(|template| {
            let haystack = format!("{} {}", template.name, template.id).to_lowercase();
            let score = words
                .iter()
                .filter(|word| haystack.contains(word.as_str()))
                .count();
            (score, template)
        })
        .filter(|(score, _)| *score > 0)
        .max_by_key(|(score, _)| *score)
        .map(|(_, template)| template)
        .unwrap_or(fallback))
}

fn draft_prompt(template: &FollowupTemplate, objective: Option<&str>) -> String {
    let mut prompt = format!(
        "Write the follow-up message for this record.\n\nTemplate (\"{}\"):\n{}",
        template.name, template.body
    );
    if let Some(objective) = objective {
        prompt.push_str(&format!(
            "\n\nWhat this follow-up needs to do:\n{objective}"
        ));
    }
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_templates_cover_the_three_common_follow_ups() {
        let ids = default_templates()
            .into_iter()
            .map(|template| template.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, ["call_followup", "proposal_chaser", "site_visit_note"]);
    }

    #[test]
    fn an_empty_template_list_restores_the_built_ins() {
        assert_eq!(
            validated_templates(Vec::new()).expect("empty is allowed"),
            default_templates()
        );
    }

    #[test]
    fn templates_are_capped_trimmed_and_uniquely_identified() {
        let too_many = (0..MAX_TEMPLATES + 1)
            .map(|index| FollowupTemplate {
                id: format!("t{index}"),
                name: format!("Template {index}"),
                body: "Body".into(),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            validated_templates(too_many)
                .expect_err("too many templates")
                .kind(),
            "invalid_input"
        );

        let blank = vec![FollowupTemplate {
            id: "t1".into(),
            name: "  ".into(),
            body: "Body".into(),
        }];
        assert_eq!(
            validated_templates(blank).expect_err("blank name").kind(),
            "invalid_input"
        );

        let long_body = vec![FollowupTemplate {
            id: "t1".into(),
            name: "Name".into(),
            body: "x".repeat(MAX_TEMPLATE_BODY_CHARS + 1),
        }];
        assert_eq!(
            validated_templates(long_body)
                .expect_err("body too long")
                .kind(),
            "invalid_input"
        );

        let duplicate = vec![
            FollowupTemplate {
                id: "t1".into(),
                name: "One".into(),
                body: "Body".into(),
            },
            FollowupTemplate {
                id: "t1".into(),
                name: "Two".into(),
                body: "Body".into(),
            },
        ];
        assert_eq!(
            validated_templates(duplicate)
                .expect_err("duplicate id")
                .kind(),
            "invalid_input"
        );

        let filled = validated_templates(vec![FollowupTemplate {
            id: "  ".into(),
            name: " Post-visit note ".into(),
            body: " Thanks for having me out. ".into(),
        }])
        .expect("valid template");
        assert_eq!(filled[0].id, "post_visit_note");
        assert_eq!(filled[0].name, "Post-visit note");
        assert_eq!(filled[0].body, "Thanks for having me out.");
    }

    #[test]
    fn the_objective_picks_the_closest_template_and_falls_back_to_the_first() {
        let templates = default_templates();
        assert_eq!(
            pick_template(&templates, None, None).expect("first").id,
            "call_followup"
        );
        assert_eq!(
            pick_template(&templates, Some("chase the proposal I sent"), None)
                .expect("matched")
                .id,
            "proposal_chaser"
        );
        assert_eq!(
            pick_template(&templates, Some("send the site visit note"), None)
                .expect("matched")
                .id,
            "site_visit_note"
        );
        // Nothing recognizable still returns a usable template.
        assert_eq!(
            pick_template(&templates, Some("zzz qqq"), None)
                .expect("fallback")
                .id,
            "call_followup"
        );
        // An explicit choice wins; an unknown one is the caller's error.
        assert_eq!(
            pick_template(
                &templates,
                Some("chase the proposal"),
                Some("call_followup")
            )
            .expect("explicit choice")
            .id,
            "call_followup"
        );
        assert_eq!(
            pick_template(&templates, None, Some("nope"))
                .expect_err("unknown template")
                .kind(),
            "not_found"
        );
    }

    #[test]
    fn a_summary_answer_splits_into_a_recap_and_bounded_actions() {
        let (summary, actions) = parse_summary(
            "Summary: Dana asked for a gate quote and went quiet.\nShe last called in June.\n\nNext actions:\n- Call Dana this week\n2. Send the revised quote\n* Log the call\n- Extra one\n- Extra two\n- Sixth action",
        );
        assert_eq!(
            summary,
            "Dana asked for a gate quote and went quiet. She last called in June."
        );
        assert_eq!(actions.len(), MAX_SUGGESTED_ACTIONS);
        assert_eq!(actions[0], "Call Dana this week");
        assert_eq!(actions[1], "Send the revised quote");
        assert_eq!(actions[2], "Log the call");
    }

    #[test]
    fn a_prose_only_answer_is_all_recap_and_never_panics() {
        let (summary, actions) = parse_summary("Nothing much has happened lately.");
        assert_eq!(summary, "Nothing much has happened lately.");
        assert!(actions.is_empty());
        assert_eq!(parse_summary(""), (String::new(), Vec::new()));
    }

    #[test]
    fn the_window_is_bounded() {
        assert_eq!(
            checked_window(None).expect("default window"),
            DEFAULT_SUMMARY_WINDOW_DAYS
        );
        assert_eq!(checked_window(Some(7)).expect("explicit window"), 7);
        assert_eq!(
            checked_window(Some(0)).expect_err("zero days").kind(),
            "invalid_input"
        );
        assert_eq!(
            checked_window(Some(MAX_SUMMARY_WINDOW_DAYS + 1))
                .expect_err("too many days")
                .kind(),
            "invalid_input"
        );
    }

    #[test]
    fn an_objective_is_trimmed_and_bounded() {
        assert_eq!(checked_objective(Some("  ")).expect("blank"), None);
        assert_eq!(
            checked_objective(Some(" chase it "))
                .expect("trimmed")
                .as_deref(),
            Some("chase it")
        );
        assert_eq!(
            checked_objective(Some(&"x".repeat(MAX_OBJECTIVE_CHARS + 1)))
                .expect_err("too long")
                .kind(),
            "invalid_input"
        );
    }
}
