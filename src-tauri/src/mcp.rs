//! MCP adapter — the local agent interface over stdio.
//!
//! This module is the whole server; `src/bin/contractorcrm-mcp.rs` only parses
//! the command line and hands it stdin/stdout, so tests drive the protocol
//! without spawning a process.
//!
//! Three rules shape everything here:
//!
//! * **No business logic.** Every tool converts JSON arguments, calls the same
//!   application/library function the desktop calls, and serializes the result.
//!   Validation, version checks, and audit rows happen where they always did.
//! * **Read-only by default.** Write tools are not even listed unless the user
//!   launched the helper with `--read-write`; calling one anyway is the
//!   `read_only` error kind, never a silent no-op.
//! * **No implicit provider calls.** The AI-backed tools reach a model only
//!   when the client invokes that tool, and they carry the same disclosure list
//!   the desktop shows. `preview_context` shows what would be sent without
//!   sending it.
//!
//! Transport is newline-delimited JSON-RPC 2.0 on stdio. No socket is ever
//! opened by this module.

use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::ai::{CompletionProvider, ContextPreview, CredentialStore, KeyringCredentialStore};
use crate::application::{
    self, ActivityPatch, CompanyPatch, ContactPatch, CreateCompanyRequest, CreateContactRequest,
    CreateOpportunityRequest, CreateTaskRequest, HandoffRefInput, LinkJobRequest, LinkQuoteRequest,
    ListTasksRequest, LogActivityRequest, MoveOpportunityStageRequest, OpportunityPatch,
    SavedViewEntityType, TaskPatch, UpdateCompanyRequest, UpdateContactRequest,
    UpdateOpportunityRequest,
};
use crate::attachments::{self, AttachmentParentType, AttachmentStore};
use crate::domain::Actor;
use crate::error::{ApplicationError, CommandError};
use crate::proposals::{
    self, ApplyProposalRequest, ProposalEntityType, ProposalStore, RecordVersion,
    UndoProposalRequest,
};
use crate::storage::{latest_migration_version, Storage};
use crate::{explain, followups, LOCAL_API_VERSION};

/// MCP revision this adapter implements. Older revisions a client may ask for
/// are accepted and echoed back, so a client is never forced to upgrade.
pub const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// Name a client sees in the handshake.
pub const SERVER_NAME: &str = "contractorcrm-mcp";

/// Bounds on what a single tool call can return. Search is capped by the
/// application layer at 50; these cover the tools that could otherwise grow.
pub const MAX_TIMELINE_ENTRIES: usize = 200;
pub const MAX_TIMELINE_BODY_CHARS: usize = 500;
pub const MAX_LIST_LIMIT: usize = 500;

/// Client name recorded in the audit log before `initialize` names one.
const UNKNOWN_CLIENT: &str = "unknown MCP client";

/// How much access the launching user granted this connection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    ReadOnly,
    ReadWrite,
}

impl Mode {
    pub fn as_wire_value(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::ReadWrite => "read_write",
        }
    }

    fn allows_writes(self) -> bool {
        matches!(self, Self::ReadWrite)
    }
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

/// One connection's state. Requests arrive serially over stdio, so a single
/// storage handle behind a mutex is all this needs.
pub struct Server {
    storage: Mutex<Storage>,
    attachments: AttachmentStore,
    credentials: Arc<dyn CredentialStore>,
    proposals: ProposalStore,
    mode: Mode,
    /// Test seam: when set, AI-backed tools use this provider instead of the
    /// configured one. The shipped binary never sets it.
    provider: Option<Arc<dyn CompletionProvider>>,
    client_name: Mutex<String>,
}

impl Server {
    /// Build a server around an already-open database — the seam tests use.
    pub fn new(
        storage: Storage,
        attachments: AttachmentStore,
        credentials: Arc<dyn CredentialStore>,
        mode: Mode,
    ) -> Self {
        Self {
            storage: Mutex::new(storage),
            attachments,
            credentials,
            proposals: ProposalStore::new(),
            mode,
            provider: None,
            client_name: Mutex::new(UNKNOWN_CLIENT.to_owned()),
        }
    }

    /// Use a canned provider for the AI-backed tools (tests only).
    pub fn with_provider(mut self, provider: Arc<dyn CompletionProvider>) -> Self {
        self.provider = Some(provider);
        self
    }

    /// Open the app's SQLite file and its attachment store beside it. Refuses
    /// a database written by a newer build rather than migrating it blindly.
    pub fn open(database_path: &std::path::Path, mode: Mode) -> Result<Self, String> {
        if !database_path.is_file() {
            return Err(format!(
                "no ContractorCRM database at {}",
                database_path.display()
            ));
        }
        let known = latest_migration_version();
        let stored = stored_migration_version(database_path)?;
        if stored > known {
            return Err(format!(
                "{} was written by a newer ContractorCRM (schema v{stored}, this helper knows v{known}); \
                 update the helper before connecting an agent",
                database_path.display()
            ));
        }

        let storage = Storage::open(database_path)
            .map_err(|error| format!("{} could not be opened: {error}", database_path.display()))?;
        // Managed attachment files live beside the database, exactly as the
        // desktop lays them out.
        let app_data = database_path
            .parent()
            .map(|parent| parent.to_path_buf())
            .unwrap_or_default();
        let attachments = AttachmentStore::open_in_app_data(app_data);
        let credentials: Arc<dyn CredentialStore> = Arc::new(KeyringCredentialStore::new());
        Ok(Self::new(storage, attachments, credentials, mode))
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Name the client gave during `initialize`, used in the audit log.
    pub fn client_name(&self) -> String {
        self.client_name
            .lock()
            .expect("client name mutex poisoned")
            .clone()
    }

    // -----------------------------------------------------------------------
    // JSON-RPC plumbing
    // -----------------------------------------------------------------------

    /// Handle one parsed message. Returns the response to write back, or
    /// `None` for a notification (which by JSON-RPC gets no reply).
    pub fn handle_message(&self, message: Value) -> Option<Value> {
        let Some(object) = message.as_object() else {
            return Some(error_response(
                Value::Null,
                -32600,
                "expected a JSON object",
            ));
        };
        // A missing id means a notification: do the work, answer nothing.
        let id = object.get("id").cloned();
        let Some(method) = object.get("method").and_then(Value::as_str) else {
            return Some(error_response(
                id.unwrap_or(Value::Null),
                -32600,
                "request has no method",
            ));
        };
        let params = object.get("params").cloned().unwrap_or(Value::Null);

        let outcome = match method {
            "initialize" => Ok(self.initialize(&params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(self.list_tools()),
            "tools/call" => self.tools_call(&params),
            // Lifecycle notifications need no reply and no state of their own.
            "notifications/initialized" | "notifications/cancelled" => Ok(json!({})),
            other => Err(RpcError::new(-32601, format!("unknown method: {other}"))),
        };

        let id = id?;
        Some(match outcome {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(error) => error_response_with_data(id, error.code, &error.message, error.data),
        })
    }

    /// Handshake: agree a protocol revision and report who we are, including
    /// the product and local API versions from docs/LOCAL_API.md "Versioning".
    fn initialize(&self, params: &Value) -> Value {
        if let Some(name) = params
            .get("clientInfo")
            .and_then(|info| info.get("name"))
            .and_then(Value::as_str)
        {
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                let mut client = self.client_name.lock().expect("client name mutex poisoned");
                *client = trimmed.chars().take(80).collect();
            }
        }

        let requested = params.get("protocolVersion").and_then(Value::as_str);
        let negotiated = match requested {
            Some(version) if SUPPORTED_PROTOCOL_VERSIONS.contains(&version) => version,
            _ => MCP_PROTOCOL_VERSION,
        };

        json!({
            "protocolVersion": negotiated,
            "capabilities": {"tools": {"listChanged": false}},
            "serverInfo": {
                "name": SERVER_NAME,
                "title": "ContractorCRM",
                "version": env!("CARGO_PKG_VERSION"),
            },
            "instructions": instructions(self.mode),
            "_meta": {
                "productVersion": env!("CARGO_PKG_VERSION"),
                "localApiVersion": LOCAL_API_VERSION,
                "mode": self.mode.as_wire_value(),
            },
        })
    }

    /// Tools this connection may use. Write tools are absent in read-only mode.
    fn list_tools(&self) -> Value {
        let tools = tools()
            .into_iter()
            .filter(|tool| self.mode.allows_writes() || !tool.write)
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "inputSchema": tool.input_schema,
                    "annotations": {"readOnlyHint": !tool.write},
                })
            })
            .collect::<Vec<_>>();
        json!({"tools": tools})
    }

    /// Run one tool. Unknown tools are JSON-RPC errors; anything the
    /// application layer rejects comes back as a tool error carrying the same
    /// stable `kind` the desktop gets.
    fn tools_call(&self, params: &Value) -> Result<Value, RpcError> {
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return Err(RpcError::new(-32602, "tools/call needs a tool name"));
        };
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));

        let Some(tool) = tools().into_iter().find(|tool| tool.name == name) else {
            return Err(RpcError::new(-32602, format!("unknown tool: {name}")));
        };
        // A write tool on a read-only connection is refused by name, so the
        // caller learns why instead of seeing a tool that does not exist.
        if tool.write && !self.mode.allows_writes() {
            return Ok(tool_error(ApplicationError::ReadOnly {
                command: name.to_owned(),
            }));
        }

        match self.call_tool(name, &arguments) {
            Ok(result) => {
                if tool.write {
                    // Audit the client behind the write; the record's own
                    // command_log row was already written by the command.
                    self.log_agent_call(name, &result);
                }
                Ok(tool_result(&result))
            }
            Err(error) => Ok(tool_error(error)),
        }
    }

    // -----------------------------------------------------------------------
    // Tools
    // -----------------------------------------------------------------------

    fn call_tool(&self, name: &str, arguments: &Value) -> Result<Value, ApplicationError> {
        match name {
            // --- reads ---
            "search_records" => {
                let args: SearchArgs = parse(arguments)?;
                let storage = self.storage();
                value(application::search_records(
                    &storage,
                    args.query,
                    args.entity_types,
                    args.limit,
                )?)
            }
            "list_contacts" => {
                let args: ListArgs = parse(arguments)?;
                let storage = self.storage();
                let contacts = application::list_contacts(&storage, args.include_archived)?;
                value(limited(contacts, args.limit)?)
            }
            "get_contact" => {
                let args: ContactIdArgs = parse(arguments)?;
                let storage = self.storage();
                value(application::get_contact(&storage, &args.contact_id)?)
            }
            "list_companies" => {
                let args: ListArgs = parse(arguments)?;
                let storage = self.storage();
                let companies = application::list_companies(&storage, args.include_archived)?;
                value(limited(companies, args.limit)?)
            }
            "get_company" => {
                let args: CompanyIdArgs = parse(arguments)?;
                let storage = self.storage();
                value(application::get_company(&storage, &args.company_id)?)
            }
            "list_opportunities" => {
                let args: ListArgs = parse(arguments)?;
                let storage = self.storage();
                let opportunities =
                    application::list_opportunities(&storage, args.include_archived)?;
                value(limited(opportunities, args.limit)?)
            }
            "get_opportunity" => {
                let args: OpportunityIdArgs = parse(arguments)?;
                let storage = self.storage();
                value(application::get_opportunity(
                    &storage,
                    &args.opportunity_id,
                )?)
            }
            "get_timeline" => {
                let args: TimelineArgs = parse(arguments)?;
                let storage = self.storage();
                let entries = application::get_timeline(
                    &storage,
                    &args.parent_type,
                    &args.parent_id,
                    args.include_related,
                )?;
                drop(storage);
                Ok(bounded_timeline(entries, args.limit, args.full_bodies)?)
            }
            "list_tasks" => {
                let args: TaskListArgs = parse(arguments)?;
                let storage = self.storage();
                let tasks = application::list_tasks(
                    &storage,
                    ListTasksRequest {
                        status: args.status,
                        overdue_only: args.overdue_only,
                        parent_type: args.parent_type,
                        parent_id: args.parent_id,
                    },
                )?;
                value(limited(tasks, args.limit)?)
            }
            "get_attention_flags" => {
                let args: AttentionArgs = parse(arguments)?;
                let storage = self.storage();
                value(application::get_attention_flags(
                    &storage,
                    args.reference_time,
                )?)
            }
            "list_saved_views" => {
                let args: EntityTypeArgs = parse(arguments)?;
                let storage = self.storage();
                value(application::list_saved_views(&storage, args.entity_type)?)
            }
            "list_tags" => {
                let args: IncludeArchivedArgs = parse(arguments)?;
                let storage = self.storage();
                value(application::list_tags(&storage, args.include_archived)?)
            }
            "list_custom_field_defs" => {
                let args: CustomFieldDefArgs = parse(arguments)?;
                let storage = self.storage();
                value(application::list_custom_field_defs(
                    &storage,
                    args.entity_type,
                    args.include_archived,
                )?)
            }
            "get_record_metadata" => {
                let args: RecordMetadataArgs = parse(arguments)?;
                let storage = self.storage();
                value(application::get_record_metadata(
                    &storage,
                    args.entity_type,
                    &args.record_id,
                )?)
            }
            "list_attachments" => {
                let args: AttachmentParentArgs = parse(arguments)?;
                let storage = self.storage();
                value(attachments::list_attachments(
                    &storage,
                    args.parent_type,
                    &args.parent_id,
                )?)
            }
            "attachment_path" => {
                let args: AttachmentIdArgs = parse(arguments)?;
                let storage = self.storage();
                value(attachments::attachment_path(
                    &storage,
                    &self.attachments,
                    &args.attachment_id,
                )?)
            }
            "get_followup_templates" => {
                let storage = self.storage();
                value(followups::get_followup_templates(&storage)?)
            }
            "preview_context" => {
                let args: PreviewArgs = parse(arguments)?;
                value(self.preview_context(&args)?)
            }

            // --- AI-backed reads (a provider call the client explicitly asked for) ---
            "summarize_history" => {
                let args: SummarizeArgs = parse(arguments)?;
                let plan = {
                    let storage = self.storage();
                    followups::plan_history_summary(
                        &storage,
                        self.credentials.as_ref(),
                        &args.parent_type,
                        &args.parent_id,
                        args.window,
                    )?
                };
                match &self.provider {
                    Some(provider) => value(plan.run_with(provider.as_ref())?),
                    None => value(plan.run()?),
                }
            }
            "explain_attention_flag" => {
                let args: FlagArgs = parse(arguments)?;
                let plan = {
                    let storage = self.storage();
                    explain::plan_explanation(
                        &storage,
                        self.credentials.as_ref(),
                        &args.flag_id,
                        None,
                    )?
                };
                match &self.provider {
                    Some(provider) => value(plan.run_with(provider.as_ref())?),
                    None => value(plan.run()?),
                }
            }
            "propose_record" => {
                let args: ProposeRecordArgs = parse(arguments)?;
                let proposal = match &self.provider {
                    Some(provider) => proposals::propose_record_with_provider(
                        &self.storage,
                        provider.as_ref(),
                        &self.proposals,
                        args.kind,
                        &args.description,
                    )?,
                    None => proposals::propose_record(
                        &self.storage,
                        self.credentials.as_ref(),
                        &self.proposals,
                        args.kind,
                        &args.description,
                    )?,
                };
                value(proposal)
            }
            "propose_update" => {
                let args: ProposeUpdateArgs = parse(arguments)?;
                let target = (
                    args.entity_type,
                    args.entity_id.as_str(),
                    args.expected_version,
                );
                let proposal = match &self.provider {
                    Some(provider) => proposals::propose_update_with_provider(
                        &self.storage,
                        provider.as_ref(),
                        &self.proposals,
                        target,
                        &args.request,
                    )?,
                    None => proposals::propose_update(
                        &self.storage,
                        self.credentials.as_ref(),
                        &self.proposals,
                        target,
                        &args.request,
                    )?,
                };
                value(proposal)
            }
            "propose_followup" => {
                let args: ProposeFollowupArgs = parse(arguments)?;
                let draft = match &self.provider {
                    Some(provider) => followups::propose_followup_with(
                        &self.storage,
                        Some(provider.as_ref()),
                        &self.proposals,
                        &args.parent_type,
                        &args.parent_id,
                        args.objective.as_deref(),
                        args.template_id.as_deref(),
                    )?,
                    None => followups::propose_followup(
                        &self.storage,
                        self.credentials.as_ref(),
                        &self.proposals,
                        &args.parent_type,
                        &args.parent_id,
                        args.objective.as_deref(),
                        args.template_id.as_deref(),
                    )?,
                };
                value(draft)
            }

            // --- writes (read-write mode only; the actor is always the agent) ---
            "apply_proposal" => {
                let args: ApplyArgs = parse(arguments)?;
                let mut storage = self.storage_mut();
                value(proposals::apply_proposal(
                    &mut storage,
                    &self.proposals,
                    ApplyProposalRequest {
                        actor: Actor::Agent,
                        proposal_id: args.proposal_id,
                        expected_versions: args.expected_versions,
                    },
                )?)
            }
            "undo_proposal" => {
                let args: UndoArgs = parse(arguments)?;
                let mut storage = self.storage_mut();
                value(proposals::undo_proposal(
                    &mut storage,
                    &self.proposals,
                    UndoProposalRequest {
                        actor: Actor::Agent,
                        undo_token: args.undo_token,
                        expected_versions: args.expected_versions,
                    },
                )?)
            }
            "create_contact" => {
                let args: CreateContactArgs = parse(arguments)?;
                let mut storage = self.storage_mut();
                value(application::create_contact(
                    &mut storage,
                    CreateContactRequest {
                        actor: Actor::Agent,
                        contact: args.contact,
                    },
                )?)
            }
            "update_contact" => {
                let args: UpdateContactArgs = parse(arguments)?;
                let mut storage = self.storage_mut();
                value(application::update_contact(
                    &mut storage,
                    UpdateContactRequest {
                        actor: Actor::Agent,
                        contact_id: args.contact_id,
                        expected_version: args.expected_version,
                        patch: args.patch,
                    },
                )?)
            }
            "create_company" => {
                let args: CreateCompanyArgs = parse(arguments)?;
                let mut storage = self.storage_mut();
                value(application::create_company(
                    &mut storage,
                    CreateCompanyRequest {
                        actor: Actor::Agent,
                        company: args.company,
                    },
                )?)
            }
            "update_company" => {
                let args: UpdateCompanyArgs = parse(arguments)?;
                let mut storage = self.storage_mut();
                value(application::update_company(
                    &mut storage,
                    UpdateCompanyRequest {
                        actor: Actor::Agent,
                        company_id: args.company_id,
                        expected_version: args.expected_version,
                        patch: args.patch,
                    },
                )?)
            }
            "create_opportunity" => {
                let args: CreateOpportunityArgs = parse(arguments)?;
                let mut storage = self.storage_mut();
                value(application::create_opportunity(
                    &mut storage,
                    CreateOpportunityRequest {
                        actor: Actor::Agent,
                        stage_id: args.stage_id,
                        opportunity: args.opportunity,
                    },
                )?)
            }
            "update_opportunity" => {
                let args: UpdateOpportunityArgs = parse(arguments)?;
                let mut storage = self.storage_mut();
                value(application::update_opportunity(
                    &mut storage,
                    UpdateOpportunityRequest {
                        actor: Actor::Agent,
                        opportunity_id: args.opportunity_id,
                        expected_version: args.expected_version,
                        patch: args.patch,
                    },
                )?)
            }
            "move_opportunity_stage" => {
                let args: MoveStageArgs = parse(arguments)?;
                let mut storage = self.storage_mut();
                value(application::move_opportunity_stage(
                    &mut storage,
                    MoveOpportunityStageRequest {
                        actor: Actor::Agent,
                        opportunity_id: args.opportunity_id,
                        to_stage_id: args.to_stage_id,
                        lost_reason_id: args.lost_reason_id,
                        expected_version: args.expected_version,
                    },
                )?)
            }
            "log_activity" => {
                let args: LogActivityArgs = parse(arguments)?;
                let mut storage = self.storage_mut();
                value(application::log_activity(
                    &mut storage,
                    LogActivityRequest {
                        actor: Actor::Agent,
                        parent_type: args.parent_type,
                        parent_id: args.parent_id,
                        activity: args.activity,
                    },
                )?)
            }
            "create_task" => {
                let args: CreateTaskArgs = parse(arguments)?;
                let mut storage = self.storage_mut();
                value(application::create_task(
                    &mut storage,
                    CreateTaskRequest {
                        actor: Actor::Agent,
                        task: args.task,
                    },
                )?)
            }
            "complete_task" => {
                let args: CompleteTaskArgs = parse(arguments)?;
                let mut storage = self.storage_mut();
                value(application::complete_task(
                    &mut storage,
                    crate::application::CompleteTaskRequest {
                        actor: Actor::Agent,
                        task_id: args.task_id,
                        expected_version: args.expected_version,
                        log_activity: args.log_activity,
                    },
                )?)
            }
            "link_quote" => {
                let args: LinkQuoteArgs = parse(arguments)?;
                let mut storage = self.storage_mut();
                value(application::link_quote(
                    &mut storage,
                    LinkQuoteRequest {
                        actor: Actor::Agent,
                        opportunity_id: args.opportunity_id,
                        expected_version: args.expected_version,
                        quote_ref: args.quote_ref,
                    },
                )?)
            }
            "link_job" => {
                let args: LinkJobArgs = parse(arguments)?;
                let mut storage = self.storage_mut();
                value(application::link_job(
                    &mut storage,
                    LinkJobRequest {
                        actor: Actor::Agent,
                        opportunity_id: args.opportunity_id,
                        expected_version: args.expected_version,
                        job_ref: args.job_ref,
                    },
                )?)
            }
            // `tools_call` already matched the name against the same table.
            other => Err(ApplicationError::NotFound {
                resource: "tool",
                id: other.to_owned(),
            }),
        }
    }

    /// What an AI-backed tool would send, without sending it.
    fn preview_context(&self, args: &PreviewArgs) -> Result<ContextPreview, ApplicationError> {
        let storage = self.storage();
        match args.tool.as_str() {
            "summarize_history" | "propose_followup" => {
                let inner: SummarizeArgs = parse(&args.arguments)?;
                followups::preview_history_context(
                    &storage,
                    &inner.parent_type,
                    &inner.parent_id,
                    inner.window,
                )
            }
            "explain_attention_flag" => {
                let inner: FlagArgs = parse(&args.arguments)?;
                explain::preview_flag_context(&storage, &inner.flag_id, None)
            }
            "propose_update" => {
                let inner: PreviewUpdateArgs = parse(&args.arguments)?;
                proposals::preview_update_context(
                    &storage,
                    inner.entity_type,
                    &inner.entity_id,
                    inner.expected_version,
                )
            }
            other => Err(ApplicationError::InvalidInput {
                field: "tool".into(),
                message: format!(
                    "{other} has no context preview; use summarize_history, propose_followup, \
                     explain_attention_flag, or propose_update"
                ),
            }),
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn storage(&self) -> std::sync::MutexGuard<'_, Storage> {
        self.storage.lock().expect("storage mutex poisoned")
    }

    fn storage_mut(&self) -> std::sync::MutexGuard<'_, Storage> {
        self.storage()
    }

    /// One extra audit row naming the MCP client behind a write. The record's
    /// own `command_log` row (actor `agent`) was written by the command itself;
    /// this is the client attribution docs/LOCAL_API.md promises.
    fn log_agent_call(&self, tool: &str, result: &Value) {
        let (entity_type, entity_id) = audit_target(tool, result);
        let summary = format!("{tool} through MCP client {}", self.client_name());
        let mut storage = self.storage_mut();
        let Ok(transaction) = application::immediate(&mut storage) else {
            return;
        };
        // Audit is best-effort after the fact: the write already committed in
        // its own transaction and must not be undone by a logging failure.
        if application::log_command(
            &transaction,
            Actor::Agent,
            entity_type,
            &entity_id,
            &summary,
        )
        .is_ok()
        {
            let _ = transaction.commit();
        }
    }
}

/// Read the highest applied migration without running any migration.
fn stored_migration_version(database_path: &std::path::Path) -> Result<i64, String> {
    let connection = rusqlite::Connection::open(database_path)
        .map_err(|error| format!("{} could not be read: {error}", database_path.display()))?;
    let version: Option<i64> = connection
        .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .unwrap_or(Some(0));
    Ok(version.unwrap_or(0))
}

/// Serve JSON-RPC messages until the client closes stdin (graceful shutdown).
pub fn serve<R: BufRead, W: Write>(
    server: &Server,
    input: R,
    mut output: W,
) -> std::io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(message) => server.handle_message(message),
            Err(error) => Some(error_response(
                Value::Null,
                -32700,
                &format!("invalid JSON: {error}"),
            )),
        };
        if let Some(response) = response {
            writeln!(output, "{response}")?;
            output.flush()?;
        }
    }
    Ok(())
}

/// The onboarding line the desktop shows and this helper answers to.
fn instructions(mode: Mode) -> String {
    match mode {
        Mode::ReadOnly => "ContractorCRM, read-only. You can look up contacts, companies, \
            opportunities, activities, tasks, and attention flags, and you can draft \
            proposals — but nothing is written. Ask the user to restart this helper with \
            --read-write to apply anything."
            .to_owned(),
        Mode::ReadWrite => "ContractorCRM, read-write. Reads are unrestricted; every write \
            is recorded in the CRM's audit log against this client. Prefer propose_* plus \
            apply_proposal so the user can undo, and always pass expectedVersion on updates."
            .to_owned(),
    }
}

/// Which record an audit row belongs to. Falls back to the tool name when a
/// result carries no id (nothing here can fail a write that already happened).
fn audit_target(tool: &str, result: &Value) -> (&'static str, String) {
    let entity_type = match tool {
        "create_contact" | "update_contact" => "contact",
        "create_company" | "update_company" => "company",
        "create_opportunity"
        | "update_opportunity"
        | "move_opportunity_stage"
        | "link_quote"
        | "link_job" => "opportunity",
        "log_activity" => "activity",
        "create_task" | "complete_task" => "task",
        _ => "proposal",
    };
    let id = result
        .get("id")
        .or_else(|| result.get("entityId"))
        .and_then(Value::as_str)
        .unwrap_or(tool)
        .to_owned();
    (entity_type, id)
}

// ---------------------------------------------------------------------------
// JSON-RPC and tool-result shapes
// ---------------------------------------------------------------------------

/// A JSON-RPC level failure (bad method, unknown tool, malformed envelope).
struct RpcError {
    code: i64,
    message: String,
    data: Option<Value>,
}

impl RpcError {
    fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    error_response_with_data(id, code, message, None)
}

fn error_response_with_data(id: Value, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut error = json!({"code": code, "message": message});
    if let Some(data) = data {
        error["data"] = data;
    }
    json!({"jsonrpc": "2.0", "id": id, "error": error})
}

/// A successful tool call: readable text plus the exact wire JSON the desktop
/// would get for the same command.
fn tool_result(result: &Value) -> Value {
    let text = serde_json::to_string_pretty(result).unwrap_or_else(|_| result.to_string());
    json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": {"result": result},
        "isError": false,
    })
}

/// A failed tool call, carrying the stable error kind and its details.
fn tool_error(error: ApplicationError) -> Value {
    let wire = CommandError::from(error);
    let payload = serde_json::to_value(&wire).unwrap_or_else(|_| json!({"kind": "io"}));
    json!({
        "content": [{"type": "text", "text": format!("{}: {}", wire.kind, wire.message)}],
        "structuredContent": {"error": payload},
        "isError": true,
    })
}

/// Serialize a command result; a serialization failure is stored-data invalid.
fn value<T: Serialize>(result: T) -> Result<Value, ApplicationError> {
    serde_json::to_value(result)
        .map_err(|error| ApplicationError::InvalidStoredData(format!("result: {error}")))
}

/// Deserialize tool arguments, turning any shape problem into `invalid_input`
/// with the field path serde reports.
fn parse<T: DeserializeOwned>(arguments: &Value) -> Result<T, ApplicationError> {
    serde_json::from_value(arguments.clone()).map_err(|error| ApplicationError::InvalidInput {
        field: "arguments".into(),
        message: error.to_string(),
    })
}

/// Apply an optional caller limit to a list result.
fn limited<T>(mut items: Vec<T>, limit: Option<usize>) -> Result<Vec<T>, ApplicationError> {
    let Some(limit) = limit else {
        return Ok(items);
    };
    if limit == 0 || limit > MAX_LIST_LIMIT {
        return Err(ApplicationError::InvalidInput {
            field: "limit".into(),
            message: format!("must be between 1 and {MAX_LIST_LIMIT}"),
        });
    }
    items.truncate(limit);
    Ok(items)
}

/// Timeline entries, capped and with bodies truncated unless the caller asked
/// for full ones. Contact history is the biggest read surface here, so it is
/// bounded even when the caller forgets to ask for a limit.
fn bounded_timeline(
    entries: Vec<crate::domain::Activity>,
    limit: Option<usize>,
    full_bodies: bool,
) -> Result<Value, ApplicationError> {
    let limit = limit.unwrap_or(MAX_TIMELINE_ENTRIES);
    if limit == 0 || limit > MAX_TIMELINE_ENTRIES {
        return Err(ApplicationError::InvalidInput {
            field: "limit".into(),
            message: format!("must be between 1 and {MAX_TIMELINE_ENTRIES}"),
        });
    }
    let mut entries = value(entries)?;
    let Some(array) = entries.as_array_mut() else {
        return Ok(entries);
    };
    array.truncate(limit);
    if !full_bodies {
        for entry in array.iter_mut() {
            truncate_field(entry, "body", MAX_TIMELINE_BODY_CHARS);
        }
    }
    Ok(entries)
}

/// Shorten one string field in place, marking that it was cut.
fn truncate_field(entry: &mut Value, field: &str, max_chars: usize) {
    let Some(text) = entry.get(field).and_then(Value::as_str) else {
        return;
    };
    if text.chars().count() <= max_chars {
        return;
    }
    let shortened = text.chars().take(max_chars).collect::<String>();
    entry[field] = Value::String(format!("{shortened}… (truncated)"));
}

// ---------------------------------------------------------------------------
// Tool arguments
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SearchArgs {
    query: String,
    #[serde(default)]
    entity_types: Option<Vec<String>>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ListArgs {
    #[serde(default)]
    include_archived: bool,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IncludeArchivedArgs {
    #[serde(default)]
    include_archived: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ContactIdArgs {
    contact_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CompanyIdArgs {
    company_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OpportunityIdArgs {
    opportunity_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TimelineArgs {
    parent_type: String,
    parent_id: String,
    #[serde(default)]
    include_related: bool,
    #[serde(default)]
    limit: Option<usize>,
    /// Bodies come back truncated unless this is set.
    #[serde(default)]
    full_bodies: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TaskListArgs {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    overdue_only: bool,
    #[serde(default)]
    parent_type: Option<String>,
    #[serde(default)]
    parent_id: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AttentionArgs {
    #[serde(default)]
    reference_time: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EntityTypeArgs {
    entity_type: SavedViewEntityType,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CustomFieldDefArgs {
    entity_type: SavedViewEntityType,
    #[serde(default)]
    include_archived: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecordMetadataArgs {
    entity_type: SavedViewEntityType,
    record_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AttachmentParentArgs {
    parent_type: AttachmentParentType,
    parent_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AttachmentIdArgs {
    attachment_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SummarizeArgs {
    parent_type: String,
    parent_id: String,
    #[serde(default)]
    window: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FlagArgs {
    flag_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProposeRecordArgs {
    kind: ProposalEntityType,
    description: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProposeUpdateArgs {
    entity_type: ProposalEntityType,
    entity_id: String,
    request: String,
    expected_version: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreviewUpdateArgs {
    entity_type: ProposalEntityType,
    entity_id: String,
    expected_version: i64,
    /// Accepted and ignored, so the same arguments work for both tools.
    #[serde(default)]
    #[allow(dead_code)]
    request: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProposeFollowupArgs {
    parent_type: String,
    parent_id: String,
    #[serde(default)]
    objective: Option<String>,
    #[serde(default)]
    template_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreviewArgs {
    tool: String,
    #[serde(default)]
    arguments: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ApplyArgs {
    proposal_id: String,
    #[serde(default)]
    expected_versions: Vec<RecordVersion>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UndoArgs {
    undo_token: String,
    #[serde(default)]
    expected_versions: Vec<RecordVersion>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateContactArgs {
    contact: ContactPatch,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateContactArgs {
    contact_id: String,
    expected_version: i64,
    patch: ContactPatch,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateCompanyArgs {
    company: CompanyPatch,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateCompanyArgs {
    company_id: String,
    expected_version: i64,
    patch: CompanyPatch,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateOpportunityArgs {
    #[serde(default)]
    stage_id: Option<String>,
    opportunity: OpportunityPatch,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateOpportunityArgs {
    opportunity_id: String,
    expected_version: i64,
    patch: OpportunityPatch,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MoveStageArgs {
    opportunity_id: String,
    to_stage_id: String,
    #[serde(default)]
    lost_reason_id: Option<String>,
    expected_version: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LogActivityArgs {
    parent_type: String,
    parent_id: String,
    activity: ActivityPatch,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateTaskArgs {
    task: TaskPatch,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CompleteTaskArgs {
    task_id: String,
    expected_version: i64,
    #[serde(default)]
    log_activity: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LinkQuoteArgs {
    opportunity_id: String,
    expected_version: i64,
    quote_ref: HandoffRefInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LinkJobArgs {
    opportunity_id: String,
    expected_version: i64,
    job_ref: HandoffRefInput,
}

// ---------------------------------------------------------------------------
// Tool table
// ---------------------------------------------------------------------------

/// One advertised tool. `write` decides whether it appears at all in read-only
/// mode and whether a successful call gets an agent audit row.
struct ToolDef {
    name: &'static str,
    description: &'static str,
    write: bool,
    input_schema: Value,
}

fn text(description: &str) -> Value {
    json!({"type": "string", "description": description})
}

fn flag(description: &str) -> Value {
    json!({"type": "boolean", "description": description})
}

fn count(description: &str) -> Value {
    json!({"type": "integer", "minimum": 1, "description": description})
}

fn record(description: &str) -> Value {
    json!({"type": "object", "description": description})
}

fn choice(values: &[&str], description: &str) -> Value {
    json!({"type": "string", "enum": values, "description": description})
}

fn schema(properties: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": properties, "required": required})
}

const RECORD_TYPES: &[&str] = &["contact", "company", "opportunity"];
const PARENT_TYPES: &[&str] = &["contact", "company", "opportunity"];

/// Every tool this adapter can expose, read tools first. The names, arguments,
/// and results mirror docs/LOCAL_API.md and `schemas/v1/local-api.json`.
fn tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "search_records",
            description: "Search contacts, companies, opportunities, and activities. \
                          One bounded page, at most 50 results.",
            write: false,
            input_schema: schema(
                json!({
                    "query": text("What to search for; plain words, not FTS syntax."),
                    "entityTypes": {
                        "type": "array",
                        "items": choice(&["contact", "company", "opportunity", "activity"], "Record type"),
                        "description": "Limit the search to these record types.",
                    },
                    "limit": json!({"type": "integer", "minimum": 1, "maximum": 50,
                        "description": "Results to return (default 25, maximum 50)."}),
                }),
                &["query"],
            ),
        },
        ToolDef {
            name: "list_contacts",
            description: "All active contacts (leads, clients, subs, vendors).",
            write: false,
            input_schema: schema(
                json!({
                    "includeArchived": flag("Include archived contacts."),
                    "limit": count("Return at most this many rows."),
                }),
                &[],
            ),
        },
        ToolDef {
            name: "get_contact",
            description: "One contact with its channels and metadata.",
            write: false,
            input_schema: schema(json!({"contactId": text("Contact id.")}), &["contactId"]),
        },
        ToolDef {
            name: "list_companies",
            description: "All active companies.",
            write: false,
            input_schema: schema(
                json!({
                    "includeArchived": flag("Include archived companies."),
                    "limit": count("Return at most this many rows."),
                }),
                &[],
            ),
        },
        ToolDef {
            name: "get_company",
            description: "One company.",
            write: false,
            input_schema: schema(json!({"companyId": text("Company id.")}), &["companyId"]),
        },
        ToolDef {
            name: "list_opportunities",
            description: "All active opportunities with their stage and links.",
            write: false,
            input_schema: schema(
                json!({
                    "includeArchived": flag("Include archived opportunities."),
                    "limit": count("Return at most this many rows."),
                }),
                &[],
            ),
        },
        ToolDef {
            name: "get_opportunity",
            description: "One opportunity with its stage history and hand-off links.",
            write: false,
            input_schema: schema(
                json!({"opportunityId": text("Opportunity id.")}),
                &["opportunityId"],
            ),
        },
        ToolDef {
            name: "get_timeline",
            description: "Logged activity on one record, newest first. Bodies are \
                          truncated unless fullBodies is set; at most 200 entries.",
            write: false,
            input_schema: schema(
                json!({
                    "parentType": choice(PARENT_TYPES, "Which record the timeline belongs to."),
                    "parentId": text("Record id."),
                    "includeRelated": flag("Also include activity on linked records."),
                    "limit": json!({"type": "integer", "minimum": 1, "maximum": 200,
                        "description": "Entries to return (default and maximum 200)."}),
                    "fullBodies": flag("Return untruncated activity bodies."),
                }),
                &["parentType", "parentId"],
            ),
        },
        ToolDef {
            name: "list_tasks",
            description: "Follow-up tasks, optionally filtered by status, parent, or overdue.",
            write: false,
            input_schema: schema(
                json!({
                    "status": text("Task status, e.g. \"open\"."),
                    "overdueOnly": flag("Only tasks past their due date."),
                    "parentType": choice(PARENT_TYPES, "Limit to one record's tasks."),
                    "parentId": text("Record id, with parentType."),
                    "limit": count("Return at most this many rows."),
                }),
                &[],
            ),
        },
        ToolDef {
            name: "get_attention_flags",
            description: "Deterministic stale-lead, overdue-task, and no-response flags.",
            write: false,
            input_schema: schema(
                json!({"referenceTime": text("UTC ISO-8601 instant to evaluate against.")}),
                &[],
            ),
        },
        ToolDef {
            name: "list_saved_views",
            description: "Saved filter/sort definitions for one list surface.",
            write: false,
            input_schema: schema(
                json!({"entityType": choice(RECORD_TYPES, "Which list surface.")}),
                &["entityType"],
            ),
        },
        ToolDef {
            name: "list_tags",
            description: "Tags available on records.",
            write: false,
            input_schema: schema(
                json!({"includeArchived": flag("Include archived tags.")}),
                &[],
            ),
        },
        ToolDef {
            name: "list_custom_field_defs",
            description: "Custom field definitions for one record type.",
            write: false,
            input_schema: schema(
                json!({
                    "entityType": choice(RECORD_TYPES, "Which record type."),
                    "includeArchived": flag("Include archived definitions."),
                }),
                &["entityType"],
            ),
        },
        ToolDef {
            name: "get_record_metadata",
            description: "Tags and custom field values on one record.",
            write: false,
            input_schema: schema(
                json!({
                    "entityType": choice(RECORD_TYPES, "Which record type."),
                    "recordId": text("Record id."),
                }),
                &["entityType", "recordId"],
            ),
        },
        ToolDef {
            name: "list_attachments",
            description: "Managed files on a contact or opportunity. File contents are \
                          never returned.",
            write: false,
            input_schema: schema(
                json!({
                    "parentType": choice(&["contact", "opportunity"], "Which record."),
                    "parentId": text("Record id."),
                }),
                &["parentType", "parentId"],
            ),
        },
        ToolDef {
            name: "attachment_path",
            description: "Absolute path of one managed file, and whether it is still on disk.",
            write: false,
            input_schema: schema(
                json!({"attachmentId": text("Attachment id.")}),
                &["attachmentId"],
            ),
        },
        ToolDef {
            name: "get_followup_templates",
            description: "The stored follow-up wordings drafting starts from.",
            write: false,
            input_schema: schema(json!({}), &[]),
        },
        ToolDef {
            name: "preview_context",
            description: "Exactly what an AI-backed tool would send a model, without \
                          sending it: the bounded projection text and the records it names.",
            write: false,
            input_schema: schema(
                json!({
                    "tool": choice(
                        &["summarize_history", "propose_followup", "explain_attention_flag", "propose_update"],
                        "Which tool to preview.",
                    ),
                    "arguments": record("The arguments you would pass to that tool."),
                }),
                &["tool", "arguments"],
            ),
        },
        ToolDef {
            name: "summarize_history",
            description: "Recap one record's recent history and suggest next actions. \
                          Sends a bounded projection to the configured model.",
            write: false,
            input_schema: schema(
                json!({
                    "parentType": choice(PARENT_TYPES, "Which record."),
                    "parentId": text("Record id."),
                    "window": json!({"type": "integer", "minimum": 1, "maximum": 3650,
                        "description": "Days of history to include (default 90)."}),
                }),
                &["parentType", "parentId"],
            ),
        },
        ToolDef {
            name: "explain_attention_flag",
            description: "Explain one current attention flag in plain language. \
                          Sends the rule, its thresholds, and the flagged record only.",
            write: false,
            input_schema: schema(
                json!({"flagId": text("Flag id from get_attention_flags, e.g. \"stale_lead:<id>\".")}),
                &["flagId"],
            ),
        },
        ToolDef {
            name: "propose_record",
            description: "Draft a new contact, company, or opportunity from a plain-language \
                          note. Returns a proposal; writes nothing.",
            write: false,
            input_schema: schema(
                json!({
                    "kind": choice(RECORD_TYPES, "What to draft."),
                    "description": text("Plain-language description of the record."),
                }),
                &["kind", "description"],
            ),
        },
        ToolDef {
            name: "propose_update",
            description: "Draft a change to an existing record. Returns a typed diff; \
                          writes nothing. expectedVersion is checked before the model is asked.",
            write: false,
            input_schema: schema(
                json!({
                    "entityType": choice(RECORD_TYPES, "Which record type."),
                    "entityId": text("Record id."),
                    "request": text("Plain-language description of the change."),
                    "expectedVersion": json!({"type": "integer",
                        "description": "The record version you read."}),
                }),
                &["entityType", "entityId", "request", "expectedVersion"],
            ),
        },
        ToolDef {
            name: "propose_followup",
            description: "Draft follow-up wording plus a follow-up task proposal. Works \
                          with the assistant off (template used verbatim). Writes nothing.",
            write: false,
            input_schema: schema(
                json!({
                    "parentType": choice(PARENT_TYPES, "Which record."),
                    "parentId": text("Record id."),
                    "objective": text("What the follow-up should accomplish."),
                    "templateId": text("Pick a stored template outright."),
                }),
                &["parentType", "parentId"],
            ),
        },
        // --- writes ---
        ToolDef {
            name: "apply_proposal",
            description: "Apply a draft: re-checks versions, re-runs validation, and \
                          returns an undo token.",
            write: true,
            input_schema: schema(
                json!({
                    "proposalId": text("Proposal id."),
                    "expectedVersions": {
                        "type": "array",
                        "items": record("{entityType, entityId, version}"),
                        "description": "Extra version assertions to check first.",
                    },
                }),
                &["proposalId"],
            ),
        },
        ToolDef {
            name: "undo_proposal",
            description: "Reverse one applied draft. A created record is archived, an \
                          updated record is restored.",
            write: true,
            input_schema: schema(
                json!({
                    "undoToken": text("Token from apply_proposal."),
                    "expectedVersions": {
                        "type": "array",
                        "items": record("{entityType, entityId, version}"),
                        "description": "Extra version assertions to check first.",
                    },
                }),
                &["undoToken"],
            ),
        },
        ToolDef {
            name: "create_contact",
            description: "Create a contact.",
            write: true,
            input_schema: schema(
                json!({"contact": record("Contact fields; kind is required.")}),
                &["contact"],
            ),
        },
        ToolDef {
            name: "update_contact",
            description: "Update a contact; version-checked.",
            write: true,
            input_schema: schema(
                json!({
                    "contactId": text("Contact id."),
                    "expectedVersion": json!({"type": "integer", "description": "The version you read."}),
                    "patch": record("Full contact field set to store."),
                }),
                &["contactId", "expectedVersion", "patch"],
            ),
        },
        ToolDef {
            name: "create_company",
            description: "Create a company.",
            write: true,
            input_schema: schema(
                json!({"company": record("Company fields; name and kind are required.")}),
                &["company"],
            ),
        },
        ToolDef {
            name: "update_company",
            description: "Update a company; version-checked.",
            write: true,
            input_schema: schema(
                json!({
                    "companyId": text("Company id."),
                    "expectedVersion": json!({"type": "integer", "description": "The version you read."}),
                    "patch": record("Full company field set to store."),
                }),
                &["companyId", "expectedVersion", "patch"],
            ),
        },
        ToolDef {
            name: "create_opportunity",
            description: "Create an opportunity in the pipeline.",
            write: true,
            input_schema: schema(
                json!({
                    "stageId": text("Starting stage; defaults to the first open stage."),
                    "opportunity": record("Opportunity fields; name and currencyCode are required."),
                }),
                &["opportunity"],
            ),
        },
        ToolDef {
            name: "update_opportunity",
            description: "Update an opportunity; version-checked.",
            write: true,
            input_schema: schema(
                json!({
                    "opportunityId": text("Opportunity id."),
                    "expectedVersion": json!({"type": "integer", "description": "The version you read."}),
                    "patch": record("Full opportunity field set to store."),
                }),
                &["opportunityId", "expectedVersion", "patch"],
            ),
        },
        ToolDef {
            name: "move_opportunity_stage",
            description: "Move an opportunity to another stage. Moving to a lost stage \
                          requires a lost reason.",
            write: true,
            input_schema: schema(
                json!({
                    "opportunityId": text("Opportunity id."),
                    "toStageId": text("Target stage id."),
                    "lostReasonId": text("Required when the target stage is a lost stage."),
                    "expectedVersion": json!({"type": "integer", "description": "The version you read."}),
                }),
                &["opportunityId", "toStageId", "expectedVersion"],
            ),
        },
        ToolDef {
            name: "log_activity",
            description: "Log a call, note, email, or meeting on a record.",
            write: true,
            input_schema: schema(
                json!({
                    "parentType": choice(PARENT_TYPES, "Which record."),
                    "parentId": text("Record id."),
                    "activity": record("Activity fields; kind and summary are required."),
                }),
                &["parentType", "parentId", "activity"],
            ),
        },
        ToolDef {
            name: "create_task",
            description: "Create a follow-up task, optionally linked to a record.",
            write: true,
            input_schema: schema(
                json!({"task": record("Task fields; title is required.")}),
                &["task"],
            ),
        },
        ToolDef {
            name: "complete_task",
            description: "Complete a task; version-checked.",
            write: true,
            input_schema: schema(
                json!({
                    "taskId": text("Task id."),
                    "expectedVersion": json!({"type": "integer", "description": "The version you read."}),
                    "logActivity": flag("Also log a note on the task's parent."),
                }),
                &["taskId", "expectedVersion"],
            ),
        },
        ToolDef {
            name: "link_quote",
            description: "Record a quote reference on an opportunity; version-checked.",
            write: true,
            input_schema: schema(
                json!({
                    "opportunityId": text("Opportunity id."),
                    "expectedVersion": json!({"type": "integer", "description": "The version you read."}),
                    "quoteRef": record("{tool, externalId, label?}"),
                }),
                &["opportunityId", "expectedVersion", "quoteRef"],
            ),
        },
        ToolDef {
            name: "link_job",
            description: "Record the ContractorProject job hand-off on an opportunity; \
                          version-checked.",
            write: true,
            input_schema: schema(
                json!({
                    "opportunityId": text("Opportunity id."),
                    "expectedVersion": json!({"type": "integer", "description": "The version you read."}),
                    "jobRef": record("{tool, externalId, label?}"),
                }),
                &["opportunityId", "expectedVersion", "jobRef"],
            ),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_mode_hides_every_write_tool() {
        let names = tools()
            .into_iter()
            .filter(|tool| tool.write)
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        assert!(names.contains(&"apply_proposal"));
        assert!(!names.contains(&"search_records"));
    }

    #[test]
    fn tool_names_are_unique() {
        let mut names = tools()
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "duplicate tool name");
    }

    #[test]
    fn a_long_activity_body_is_truncated_with_a_marker() {
        let mut entry = json!({"body": "x".repeat(MAX_TIMELINE_BODY_CHARS + 10)});
        truncate_field(&mut entry, "body", MAX_TIMELINE_BODY_CHARS);
        let body = entry["body"].as_str().expect("body text");
        assert!(body.ends_with("… (truncated)"));
        assert_eq!(
            body.chars().count(),
            MAX_TIMELINE_BODY_CHARS + "… (truncated)".chars().count()
        );
    }
}
