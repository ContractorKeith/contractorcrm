//! Pure needs-attention rules (docs/ARCHITECTURE.md): given plain facts and a
//! reference time, return deterministic attention flags. This module must not
//! know about React, SQLite, Tauri, or AI — std, serde, and chrono only. The
//! application layer gathers the facts; the model may explain these flags but
//! never invents them.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// Rule thresholds; persisted in app_settings by the application layer and
/// read with these defaults when absent.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Thresholds {
    /// A lead with no touch in this many days is stale.
    pub stale_lead_days: i64,
    /// Days in the proposal stage without an inbound response before flagging.
    pub proposal_no_response_days: i64,
    /// Name of the open stage that means "proposal sent" (user-renamable).
    pub proposal_stage_name: String,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            stale_lead_days: 21,
            proposal_no_response_days: 7,
            proposal_stage_name: "Proposal Sent".into(),
        }
    }
}

/// Which deterministic rule produced a flag.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionRule {
    OverdueTask,
    ProposalNoResponse,
    StaleLead,
}

impl AttentionRule {
    /// Stable id prefix; flag ids are `<prefix>:<record id>`.
    fn id_prefix(self) -> &'static str {
        match self {
            Self::OverdueTask => "overdue_task",
            Self::ProposalNoResponse => "proposal_no_response",
            Self::StaleLead => "stale_lead",
        }
    }

    /// Severity order: overdue tasks first (an explicit commitment already
    /// missed), then proposals without response (live money waiting on a
    /// follow-up), then stale leads (the softest signal — nothing promised,
    /// just silence). Lower ranks sort first.
    fn severity_rank(self) -> u8 {
        match self {
            Self::OverdueTask => 0,
            Self::ProposalNoResponse => 1,
            Self::StaleLead => 2,
        }
    }
}

/// Which record type a flag points back to.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionRecordType {
    Contact,
    Opportunity,
    Task,
}

/// Contact facts the rules need — nothing more.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContactFacts {
    pub id: String,
    pub display_name: String,
    /// Party kind wire value, e.g. "lead"; only leads go stale.
    pub kind: String,
    pub created_at: DateTime<Utc>,
    /// Latest activity on the contact or any of its opportunities.
    pub last_activity_at: Option<DateTime<Utc>>,
}

/// Opportunity facts the rules need — current stage plus response timing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpportunityFacts {
    pub id: String,
    pub name: String,
    /// Stage kind wire value: "open", "won", or "lost".
    pub stage_kind: String,
    pub stage_name: String,
    /// When the opportunity entered its current stage (from stage_history).
    pub stage_entered_at: DateTime<Utc>,
    /// Latest inbound activity logged on the opportunity.
    pub last_inbound_activity_at: Option<DateTime<Utc>>,
}

/// Task facts the rules need — status and due date only.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskFacts {
    pub id: String,
    pub title: String,
    /// Task status wire value; only "open" tasks can be overdue.
    pub status: String,
    pub due_at: Option<DateTime<Utc>>,
}

/// Everything `evaluate` looks at — no clocks, no storage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttentionInputs {
    pub reference_time: DateTime<Utc>,
    pub thresholds: Thresholds,
    pub contacts: Vec<ContactFacts>,
    pub opportunities: Vec<OpportunityFacts>,
    pub tasks: Vec<TaskFacts>,
}

/// One deterministic attention flag; never stored, recomputed on demand.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttentionFlag {
    /// Stable id derived from rule + record id, e.g. "overdue_task:abc".
    pub id: String,
    pub rule: AttentionRule,
    pub record_type: AttentionRecordType,
    pub record_id: String,
    pub record_display_name: String,
    /// Plain-language reason the flag exists.
    pub explanation: String,
}

/// Run every rule over the inputs. Output is deterministically ordered by
/// severity (see `AttentionRule::severity_rank`), then by flag id.
pub fn evaluate(inputs: &AttentionInputs) -> Vec<AttentionFlag> {
    let mut flags = Vec::new();
    flags.extend(overdue_task_flags(inputs));
    flags.extend(proposal_no_response_flags(inputs));
    flags.extend(stale_lead_flags(inputs));
    flags.sort_by(|a, b| {
        a.rule
            .severity_rank()
            .cmp(&b.rule.severity_rank())
            .then_with(|| a.id.cmp(&b.id))
    });
    flags
}

/// Open task with a due date strictly before the reference time.
fn overdue_task_flags(inputs: &AttentionInputs) -> Vec<AttentionFlag> {
    inputs
        .tasks
        .iter()
        .filter_map(|task| {
            let due_at = task.due_at?;
            if task.status != "open" || due_at >= inputs.reference_time {
                return None;
            }
            let days_overdue = (inputs.reference_time - due_at).num_days();
            let explanation = if days_overdue < 1 {
                format!("Task \"{}\" is past due today.", task.title)
            } else {
                format!("Task \"{}\" is {days_overdue} day(s) overdue.", task.title)
            };
            Some(flag(
                AttentionRule::OverdueTask,
                AttentionRecordType::Task,
                &task.id,
                &task.title,
                explanation,
            ))
        })
        .collect()
}

/// Open opportunity sitting in the proposal stage for at least the threshold
/// with no inbound activity since entering it.
fn proposal_no_response_flags(inputs: &AttentionInputs) -> Vec<AttentionFlag> {
    let thresholds = &inputs.thresholds;
    inputs
        .opportunities
        .iter()
        .filter_map(|opportunity| {
            if opportunity.stage_kind != "open"
                || opportunity.stage_name != thresholds.proposal_stage_name
            {
                return None;
            }
            // Threshold reached exactly at N days in the stage.
            let flags_at =
                opportunity.stage_entered_at + Duration::days(thresholds.proposal_no_response_days);
            if inputs.reference_time < flags_at {
                return None;
            }
            // Any inbound touch at or after entering the stage counts as a response.
            if let Some(inbound_at) = opportunity.last_inbound_activity_at {
                if inbound_at >= opportunity.stage_entered_at {
                    return None;
                }
            }
            let days_waiting = (inputs.reference_time - opportunity.stage_entered_at).num_days();
            Some(flag(
                AttentionRule::ProposalNoResponse,
                AttentionRecordType::Opportunity,
                &opportunity.id,
                &opportunity.name,
                format!(
                    "\"{}\" has sat in {} for {days_waiting} day(s) with no inbound response.",
                    opportunity.name, opportunity.stage_name
                ),
            ))
        })
        .collect()
}

/// Lead with no touch (own or related-opportunity activity) in the threshold
/// window. A lead with no activity at all is measured from its created_at, so
/// brand-new leads are not instantly stale.
fn stale_lead_flags(inputs: &AttentionInputs) -> Vec<AttentionFlag> {
    let thresholds = &inputs.thresholds;
    inputs
        .contacts
        .iter()
        .filter_map(|contact| {
            if contact.kind != "lead" {
                return None;
            }
            let last_touch = contact.last_activity_at.unwrap_or(contact.created_at);
            // Threshold reached exactly at N days of silence.
            let flags_at = last_touch + Duration::days(thresholds.stale_lead_days);
            if inputs.reference_time < flags_at {
                return None;
            }
            let days_quiet = (inputs.reference_time - last_touch).num_days();
            Some(flag(
                AttentionRule::StaleLead,
                AttentionRecordType::Contact,
                &contact.id,
                &contact.display_name,
                format!(
                    "Lead \"{}\" has had no activity in {days_quiet} day(s).",
                    contact.display_name
                ),
            ))
        })
        .collect()
}

/// Assemble one flag with its deterministic id.
fn flag(
    rule: AttentionRule,
    record_type: AttentionRecordType,
    record_id: &str,
    record_display_name: &str,
    explanation: String,
) -> AttentionFlag {
    AttentionFlag {
        id: format!("{}:{record_id}", rule.id_prefix()),
        rule,
        record_type,
        record_id: record_id.into(),
        record_display_name: record_display_name.into(),
        explanation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse a UTC ISO-8601 test timestamp.
    fn ts(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .expect("valid test timestamp")
            .with_timezone(&Utc)
    }

    fn reference() -> DateTime<Utc> {
        ts("2026-08-14T12:00:00Z")
    }

    fn empty_inputs() -> AttentionInputs {
        AttentionInputs {
            reference_time: reference(),
            thresholds: Thresholds::default(),
            contacts: Vec::new(),
            opportunities: Vec::new(),
            tasks: Vec::new(),
        }
    }

    fn lead(id: &str, created_at: &str, last_activity_at: Option<&str>) -> ContactFacts {
        ContactFacts {
            id: id.into(),
            display_name: format!("Lead {id}"),
            kind: "lead".into(),
            created_at: ts(created_at),
            last_activity_at: last_activity_at.map(ts),
        }
    }

    fn proposal(id: &str, entered_at: &str, inbound_at: Option<&str>) -> OpportunityFacts {
        OpportunityFacts {
            id: id.into(),
            name: format!("Opp {id}"),
            stage_kind: "open".into(),
            stage_name: "Proposal Sent".into(),
            stage_entered_at: ts(entered_at),
            last_inbound_activity_at: inbound_at.map(ts),
        }
    }

    fn open_task(id: &str, due_at: Option<&str>) -> TaskFacts {
        TaskFacts {
            id: id.into(),
            title: format!("Task {id}"),
            status: "open".into(),
            due_at: due_at.map(ts),
        }
    }

    // -- overdue_task ------------------------------------------------------

    #[test]
    fn overdue_task_fires_past_due_but_not_at_or_after_the_due_moment() {
        let mut inputs = empty_inputs();
        inputs.tasks = vec![
            open_task("past", Some("2026-08-11T12:00:00Z")),
            open_task("at-reference", Some("2026-08-14T12:00:00Z")),
            open_task("future", Some("2026-08-15T12:00:00Z")),
            open_task("no-due", None),
            TaskFacts {
                status: "done".into(),
                ..open_task("done-past", Some("2026-08-11T12:00:00Z"))
            },
        ];

        let flags = evaluate(&inputs);
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].id, "overdue_task:past");
        assert_eq!(flags[0].rule, AttentionRule::OverdueTask);
        assert_eq!(flags[0].record_type, AttentionRecordType::Task);
        assert!(flags[0].explanation.contains("3 day(s) overdue"));
    }

    #[test]
    fn overdue_task_less_than_a_day_late_says_past_due_today() {
        let mut inputs = empty_inputs();
        inputs.tasks = vec![open_task("late-today", Some("2026-08-14T09:00:00Z"))];

        let flags = evaluate(&inputs);
        assert_eq!(flags.len(), 1);
        assert!(flags[0].explanation.contains("past due today"));
    }

    // -- proposal_no_response ----------------------------------------------

    #[test]
    fn proposal_fires_at_the_threshold_boundary_but_not_just_under() {
        let mut inputs = empty_inputs();
        inputs.opportunities = vec![
            proposal("exactly-7d", "2026-08-07T12:00:00Z", None),
            proposal("just-under", "2026-08-07T12:00:01Z", None),
        ];

        let flags = evaluate(&inputs);
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].id, "proposal_no_response:exactly-7d");
        assert!(flags[0].explanation.contains("Proposal Sent"));
    }

    #[test]
    fn proposal_with_an_inbound_response_after_entering_does_not_fire() {
        let mut inputs = empty_inputs();
        inputs.opportunities = vec![
            proposal(
                "answered",
                "2026-08-01T12:00:00Z",
                Some("2026-08-02T12:00:00Z"),
            ),
            proposal(
                "old-inbound",
                "2026-08-01T12:00:00Z",
                Some("2026-07-20T12:00:00Z"),
            ),
        ];

        let flags = evaluate(&inputs);
        // Inbound before entering the stage is not a response to this proposal.
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].id, "proposal_no_response:old-inbound");
    }

    #[test]
    fn proposal_rule_ignores_other_stages_and_non_open_kinds() {
        let mut inputs = empty_inputs();
        inputs.opportunities = vec![
            OpportunityFacts {
                stage_name: "Estimating".into(),
                ..proposal("other-stage", "2026-08-01T12:00:00Z", None)
            },
            OpportunityFacts {
                stage_kind: "won".into(),
                ..proposal("won", "2026-08-01T12:00:00Z", None)
            },
        ];

        assert!(evaluate(&inputs).is_empty());
    }

    // -- stale_lead --------------------------------------------------------

    #[test]
    fn stale_lead_fires_at_the_threshold_boundary_but_not_just_under() {
        let mut inputs = empty_inputs();
        inputs.contacts = vec![
            lead(
                "exactly-21d",
                "2026-01-01T00:00:00Z",
                Some("2026-07-24T12:00:00Z"),
            ),
            lead(
                "just-under",
                "2026-01-01T00:00:00Z",
                Some("2026-07-24T12:00:01Z"),
            ),
        ];

        let flags = evaluate(&inputs);
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].id, "stale_lead:exactly-21d");
        assert!(flags[0].explanation.contains("21 day(s)"));
    }

    #[test]
    fn lead_with_no_activity_is_measured_from_created_at() {
        let mut inputs = empty_inputs();
        inputs.contacts = vec![
            lead("fresh", "2026-08-13T12:00:00Z", None),
            lead("forgotten", "2026-06-01T12:00:00Z", None),
        ];

        let flags = evaluate(&inputs);
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].id, "stale_lead:forgotten");
    }

    #[test]
    fn non_leads_never_go_stale() {
        let mut inputs = empty_inputs();
        inputs.contacts = vec![ContactFacts {
            kind: "client".into(),
            ..lead("client", "2026-01-01T00:00:00Z", None)
        }];

        assert!(evaluate(&inputs).is_empty());
    }

    // -- determinism and ordering ------------------------------------------

    #[test]
    fn flag_ids_are_deterministic_across_runs() {
        let mut inputs = empty_inputs();
        inputs.contacts = vec![lead("l1", "2026-01-01T00:00:00Z", None)];
        inputs.opportunities = vec![proposal("o1", "2026-08-01T12:00:00Z", None)];
        inputs.tasks = vec![open_task("t1", Some("2026-08-10T12:00:00Z"))];

        assert_eq!(evaluate(&inputs), evaluate(&inputs));
        assert_eq!(evaluate(&inputs)[0].id, "overdue_task:t1");
    }

    #[test]
    fn flags_order_overdue_then_proposal_then_stale() {
        let mut inputs = empty_inputs();
        inputs.contacts = vec![lead("l1", "2026-01-01T00:00:00Z", None)];
        inputs.opportunities = vec![proposal("o1", "2026-08-01T12:00:00Z", None)];
        inputs.tasks = vec![
            open_task("t2", Some("2026-08-10T12:00:00Z")),
            open_task("t1", Some("2026-08-12T12:00:00Z")),
        ];

        let rules_then_ids: Vec<(AttentionRule, String)> = evaluate(&inputs)
            .into_iter()
            .map(|flag| (flag.rule, flag.id))
            .collect();
        assert_eq!(
            rules_then_ids,
            vec![
                // Same-severity flags tie-break on the stable flag id.
                (AttentionRule::OverdueTask, "overdue_task:t1".into()),
                (AttentionRule::OverdueTask, "overdue_task:t2".into()),
                (
                    AttentionRule::ProposalNoResponse,
                    "proposal_no_response:o1".into()
                ),
                (AttentionRule::StaleLead, "stale_lead:l1".into()),
            ]
        );
    }
}
