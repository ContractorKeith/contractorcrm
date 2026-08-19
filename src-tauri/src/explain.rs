//! Plain-language explanations of deterministic attention flags.
//!
//! The rules in `attention.rs` decide what needs attention; this module only
//! explains one of those findings and never adds to it. `attention.rs` knows
//! nothing about AI — the explanation layer consumes its output here.
//!
//! Two invariants shape the code:
//!
//! * **Bounded projection.** Only the flagged record's identity plus the rule,
//!   its thresholds, and the dates that tripped it leave the machine. No other
//!   contact's data, no timelines, no attachments, no credentials.
//! * **Mutex rule.** Nothing here calls the network. `plan_explanation` runs
//!   under the storage lock and hands back a self-owned plan; the caller drops
//!   the guard and only then calls `run` (see the module doc in `ai.rs`).

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::ai::{
    self, CompletionProvider, CredentialStore, OpenAiCompatibleProvider, ProviderCompletion,
    ProviderRequest, RecordRef,
};
use crate::application;
use crate::attention::{self, AttentionFlag, AttentionInputs, AttentionRecordType, AttentionRule};
use crate::error::ApplicationError;
use crate::storage::Storage;

/// Feature name carried on every request this module builds.
pub const EXPLAIN_PURPOSE: &str = "explain_attention_flag";

/// House rules for the model: explain the deterministic finding, never extend it.
const SYSTEM_TEXT: &str = "You help a contractor work their CRM. Explain plainly why this \
record needs attention and what to do next, in two or three short sentences. Use only the \
facts in the context; never invent facts, names, dates, or amounts that are not there.";

/// The ask, kept separate from the projected facts.
const USER_TEXT: &str =
    "Explain why this needs attention and suggest the next step, in plain language.";

/// Explanations are short by design — this is a nudge, not a report.
const MAX_OUTPUT_TOKENS: u32 = 400;

/// Wire shape returned to the UI: the flag it belongs to, where the request
/// went, and the completion with its own disclosure list.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttentionExplanation {
    pub flag_id: String,
    /// Host (with port) the request was sent to, for the disclosure line.
    pub endpoint_host: String,
    /// True when that host is on this machine — no data left the device.
    pub local: bool,
    pub explanation: ProviderCompletion,
}

/// A prepared explanation call: everything read from storage, nothing sent.
pub struct ExplanationPlan {
    flag_id: String,
    endpoint_host: String,
    local: bool,
    provider: OpenAiCompatibleProvider,
    request: ProviderRequest,
}

/// Debug by hand: the provider holds the API key and must never be printed.
impl std::fmt::Debug for ExplanationPlan {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ExplanationPlan")
            .field("flag_id", &self.flag_id)
            .field("endpoint_host", &self.endpoint_host)
            .field("local", &self.local)
            .field("request", &self.request)
            .finish_non_exhaustive()
    }
}

impl ExplanationPlan {
    /// The exact request that would be sent — inspectable before it goes out.
    pub fn request(&self) -> &ProviderRequest {
        &self.request
    }

    /// Send the planned call to the configured provider. Call this only after
    /// the storage lock has been released.
    pub fn run(&self) -> Result<AttentionExplanation, ApplicationError> {
        self.run_with(&self.provider)
    }

    /// Same, against any provider — the seam tests use a canned one.
    pub fn run_with(
        &self,
        provider: &dyn CompletionProvider,
    ) -> Result<AttentionExplanation, ApplicationError> {
        Ok(AttentionExplanation {
            flag_id: self.flag_id.clone(),
            endpoint_host: self.endpoint_host.clone(),
            local: self.local,
            explanation: provider.complete(&self.request)?,
        })
    }
}

/// Re-evaluate the current flags, find `flag_id`, and build the bounded
/// projection for it. A flag that no longer exists (already handled, or from
/// an older screen) is `not_found`; a switched-off or unconfigured assistant
/// is `provider_unavailable` and reads no credentials.
pub fn plan_explanation(
    storage: &Storage,
    credentials: &dyn CredentialStore,
    flag_id: &str,
    reference_time: Option<String>,
) -> Result<ExplanationPlan, ApplicationError> {
    // Settings first: an assistant that is off must not read the credential
    // store, and must not send anything anywhere.
    let Some(provider) = ai::configured_provider(storage, credentials)? else {
        return Err(ApplicationError::ProviderUnavailable {
            reason: "The AI assistant is off. Turn it on in Settings to explain a flag.".into(),
        });
    };

    let flag_id = flag_id.trim();
    if flag_id.is_empty() {
        return Err(ApplicationError::InvalidInput {
            field: "flagId".into(),
            message: "is required".into(),
        });
    }

    let inputs = application::attention_inputs(storage, reference_time)?;
    let flag = attention::evaluate(&inputs)
        .into_iter()
        .find(|candidate| candidate.id == flag_id)
        .ok_or_else(|| ApplicationError::NotFound {
            resource: "attention flag",
            id: flag_id.to_owned(),
        })?;

    let request = ProviderRequest {
        purpose: EXPLAIN_PURPOSE.into(),
        system_text: SYSTEM_TEXT.into(),
        user_text: USER_TEXT.into(),
        context_text: Some(projection(&inputs, &flag)),
        included_record_refs: vec![record_ref(&flag)],
        max_output_tokens: Some(MAX_OUTPUT_TOKENS),
        timeout_seconds: None,
    };

    Ok(ExplanationPlan {
        flag_id: flag.id,
        endpoint_host: ai::endpoint_host(provider.base_url()),
        local: ai::is_local_endpoint(provider.base_url()),
        provider,
        request,
    })
}

/// The bounded projection: the rule, its thresholds, the dates that tripped
/// it, and the one flagged record. Nothing else in the database is read into
/// this text.
fn projection(inputs: &AttentionInputs, flag: &AttentionFlag) -> String {
    let mut lines = vec![
        format!("Rule: {}", rule_label(flag.rule)),
        format!("Today: {}", day(inputs.reference_time)),
    ];
    let thresholds = &inputs.thresholds;

    match flag.rule {
        AttentionRule::OverdueTask => {
            lines.push("Threshold: an open task is overdue once its due date passes.".into());
            if let Some(task) = inputs.tasks.iter().find(|task| task.id == flag.record_id) {
                lines.push(format!("Task: {}", task.title));
                lines.push(format!("Status: {}", task.status));
                lines.push(match task.due_at {
                    Some(due_at) => format!("Due: {}", day(due_at)),
                    None => "Due: none".into(),
                });
            }
        }
        AttentionRule::ProposalNoResponse => {
            lines.push(format!(
                "Threshold: {} day(s) in the \"{}\" stage with no inbound reply.",
                thresholds.proposal_no_response_days, thresholds.proposal_stage_name
            ));
            if let Some(opportunity) = inputs
                .opportunities
                .iter()
                .find(|opportunity| opportunity.id == flag.record_id)
            {
                lines.push(format!("Opportunity: {}", opportunity.name));
                lines.push(format!(
                    "Stage: {} (entered {})",
                    opportunity.stage_name,
                    day(opportunity.stage_entered_at)
                ));
                lines.push(match opportunity.last_inbound_activity_at {
                    Some(inbound_at) => format!("Last reply from them: {}", day(inbound_at)),
                    None => "Last reply from them: none logged".into(),
                });
            }
        }
        AttentionRule::StaleLead => {
            lines.push(format!(
                "Threshold: {} day(s) with no activity on a lead.",
                thresholds.stale_lead_days
            ));
            if let Some(contact) = inputs
                .contacts
                .iter()
                .find(|contact| contact.id == flag.record_id)
            {
                lines.push(format!("Lead: {}", contact.display_name));
                lines.push(match contact.last_activity_at {
                    Some(last_activity_at) => {
                        format!("Last activity: {}", day(last_activity_at))
                    }
                    None => format!(
                        "Last activity: none logged; lead added {}",
                        day(contact.created_at)
                    ),
                });
            }
        }
    }

    lines.push(format!(
        "Record: {} ({} {})",
        flag.record_display_name,
        record_type_label(flag.record_type),
        flag.record_id
    ));
    lines.push(format!("Deterministic finding: {}", flag.explanation));
    lines.join("\n")
}

/// The one record whose data is in the projection — the disclosure list.
fn record_ref(flag: &AttentionFlag) -> RecordRef {
    RecordRef {
        entity_type: record_type_label(flag.record_type).to_owned(),
        entity_id: flag.record_id.clone(),
        label: flag.record_display_name.clone(),
    }
}

/// Contractor-facing name for a rule.
fn rule_label(rule: AttentionRule) -> &'static str {
    match rule {
        AttentionRule::OverdueTask => "overdue task",
        AttentionRule::ProposalNoResponse => "proposal with no response",
        AttentionRule::StaleLead => "stale lead",
    }
}

/// Wire value for a flagged record's type, matching the attention flag shape.
fn record_type_label(record_type: AttentionRecordType) -> &'static str {
    match record_type {
        AttentionRecordType::Contact => "contact",
        AttentionRecordType::Opportunity => "opportunity",
        AttentionRecordType::Task => "task",
    }
}

/// Calendar day of a timestamp; the rules work in days, so times add noise.
fn day(timestamp: DateTime<Utc>) -> String {
    timestamp.format("%Y-%m-%d").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attention::{ContactFacts, TaskFacts, Thresholds};

    fn ts(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .expect("valid test timestamp")
            .with_timezone(&Utc)
    }

    fn inputs_with_two_leads() -> AttentionInputs {
        AttentionInputs {
            reference_time: ts("2026-08-14T12:00:00Z"),
            thresholds: Thresholds::default(),
            contacts: vec![
                ContactFacts {
                    id: "contact-1".into(),
                    display_name: "Dana Ruiz".into(),
                    kind: "lead".into(),
                    created_at: ts("2026-05-01T12:00:00Z"),
                    last_activity_at: Some(ts("2026-06-01T12:00:00Z")),
                },
                ContactFacts {
                    id: "contact-2".into(),
                    display_name: "Marco Silva".into(),
                    kind: "lead".into(),
                    created_at: ts("2026-05-01T12:00:00Z"),
                    last_activity_at: Some(ts("2026-06-02T12:00:00Z")),
                },
            ],
            opportunities: Vec::new(),
            tasks: Vec::new(),
        }
    }

    #[test]
    fn stale_lead_projection_carries_the_rule_threshold_and_dates_only() {
        let inputs = inputs_with_two_leads();
        let flag = attention::evaluate(&inputs)
            .into_iter()
            .find(|flag| flag.record_id == "contact-1")
            .expect("stale lead flag");

        let context = projection(&inputs, &flag);

        assert!(context.contains("Rule: stale lead"));
        assert!(context.contains("Threshold: 21 day(s)"));
        assert!(context.contains("Last activity: 2026-06-01"));
        assert!(context.contains("Today: 2026-08-14"));
        assert!(context.contains("Record: Dana Ruiz (contact contact-1)"));
        // Other leads on the same screen stay on this machine.
        assert!(!context.contains("Marco Silva"));
        assert!(!context.contains("contact-2"));
    }

    #[test]
    fn a_lead_with_no_activity_is_projected_from_its_created_date() {
        let mut inputs = inputs_with_two_leads();
        inputs.contacts[0].last_activity_at = None;
        let flag = attention::evaluate(&inputs)
            .into_iter()
            .find(|flag| flag.record_id == "contact-1")
            .expect("stale lead flag");

        assert!(projection(&inputs, &flag).contains("none logged; lead added 2026-05-01"));
    }

    #[test]
    fn overdue_task_projection_names_the_task_and_its_due_date() {
        let inputs = AttentionInputs {
            reference_time: ts("2026-08-14T12:00:00Z"),
            thresholds: Thresholds::default(),
            contacts: Vec::new(),
            opportunities: Vec::new(),
            tasks: vec![TaskFacts {
                id: "task-1".into(),
                title: "Call the county office".into(),
                status: "open".into(),
                due_at: Some(ts("2026-08-11T12:00:00Z")),
            }],
        };
        let flag = attention::evaluate(&inputs)
            .into_iter()
            .next()
            .expect("overdue task flag");

        let context = projection(&inputs, &flag);
        assert!(context.contains("Rule: overdue task"));
        assert!(context.contains("Task: Call the county office"));
        assert!(context.contains("Due: 2026-08-11"));
        assert!(context.contains("Record: Call the county office (task task-1)"));
    }

    #[test]
    fn the_disclosure_list_names_exactly_the_flagged_record() {
        let inputs = inputs_with_two_leads();
        let flag = attention::evaluate(&inputs)
            .into_iter()
            .find(|flag| flag.record_id == "contact-1")
            .expect("stale lead flag");

        assert_eq!(
            record_ref(&flag),
            RecordRef {
                entity_type: "contact".into(),
                entity_id: "contact-1".into(),
                label: "Dana Ruiz".into(),
            }
        );
    }
}
