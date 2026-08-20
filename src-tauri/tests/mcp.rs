//! Integration tests for the MCP stdio adapter: the handshake, the mode
//! split, error mapping, bounded reads, the propose → apply → undo round trip,
//! and the audit trail. No network and no real keychain are involved — the
//! provider is a canned one and credentials live in memory.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use chrono::{Duration, SecondsFormat, Utc};
use contractorcrm_lib::ai::{
    set_ai_settings, CompletionProvider, CredentialStore, InMemoryCredentialStore, ProviderCheck,
    ProviderCompletion, ProviderRequest, SetAiSettingsRequest,
};
use contractorcrm_lib::application::{
    self, ActivityPatch, CompanyPatch, ContactPatch, CreateCompanyRequest, CreateContactRequest,
    CreateOpportunityRequest, CreateTaskRequest, LogActivityRequest, OpportunityPatch, TaskPatch,
};
use contractorcrm_lib::attachments::{
    self, AddAttachmentRequest, AttachmentParentType, AttachmentStore,
};
use contractorcrm_lib::domain::{Actor, Contact, StageKind};
use contractorcrm_lib::error::ApplicationError;
use contractorcrm_lib::mcp::{
    Mode, Server, MAX_LIST_LIMIT, MAX_MESSAGE_BYTES, MAX_TIMELINE_BODY_CHARS,
};
use contractorcrm_lib::storage::Storage;
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

const LOCAL_API_SCHEMA: &str = include_str!("../../schemas/v1/local-api.json");

/// Every tool the adapter advertises in read-write mode, in table order.
/// docs/SLICE5_COVERAGE.md maps each of these to its docs and its test.
const ALL_TOOLS: [&str; 39] = [
    "search_records",
    "list_contacts",
    "get_contact",
    "list_companies",
    "get_company",
    "list_opportunities",
    "get_opportunity",
    "list_stages",
    "list_lost_reasons",
    "get_timeline",
    "list_tasks",
    "get_attention_flags",
    "list_saved_views",
    "list_tags",
    "list_custom_field_defs",
    "get_record_metadata",
    "list_attachments",
    "attachment_path",
    "get_followup_templates",
    "preview_context",
    "summarize_history",
    "explain_attention_flag",
    "propose_record",
    "propose_update",
    "propose_followup",
    "apply_proposal",
    "undo_proposal",
    "create_contact",
    "update_contact",
    "create_company",
    "update_company",
    "create_opportunity",
    "update_opportunity",
    "move_opportunity_stage",
    "log_activity",
    "create_task",
    "complete_task",
    "link_quote",
    "link_job",
];

/// A provider that answers with canned text and records what it was asked.
struct CannedProvider {
    text: String,
    seen: Mutex<Vec<ProviderRequest>>,
}

impl CannedProvider {
    fn new(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            seen: Mutex::new(Vec::new()),
        }
    }

    fn call_count(&self) -> usize {
        self.seen.lock().expect("canned provider mutex").len()
    }
}

impl CompletionProvider for CannedProvider {
    fn label(&self) -> &str {
        "Canned model"
    }

    fn complete(&self, request: &ProviderRequest) -> Result<ProviderCompletion, ApplicationError> {
        self.seen
            .lock()
            .expect("canned provider mutex")
            .push(request.clone());
        Ok(ProviderCompletion {
            purpose: request.purpose.clone(),
            model: "canned-model".into(),
            text: self.text.clone(),
            included_record_refs: request.included_record_refs.clone(),
        })
    }

    fn check(&self) -> Result<ProviderCheck, ApplicationError> {
        unreachable!("the MCP adapter never runs a connection check")
    }
}

fn open_storage(temp: &tempfile::TempDir) -> Storage {
    Storage::open_in_app_data(temp.path()).expect("open storage")
}

/// A server over a fresh database in `temp`, with in-memory credentials.
fn server(temp: &tempfile::TempDir, storage: Storage, mode: Mode) -> Server {
    let credentials: Arc<dyn CredentialStore> = Arc::new(InMemoryCredentialStore::with_key("sk-x"));
    Server::new(
        storage,
        AttachmentStore::open_in_app_data(temp.path()),
        credentials,
        mode,
    )
}

fn turn_the_assistant_on(storage: &mut Storage) {
    let credentials = InMemoryCredentialStore::with_key("sk-x");
    set_ai_settings(
        storage,
        &credentials,
        SetAiSettingsRequest {
            actor: Actor::User,
            enabled: true,
            provider_label: "Local model".into(),
            base_url: "http://127.0.0.1:11434/v1".into(),
            model: "llama3.1".into(),
        },
    )
    .expect("enable the assistant");
}

fn make_contact(storage: &mut Storage, display_name: &str) -> Contact {
    application::create_contact(
        storage,
        CreateContactRequest {
            actor: Actor::User,
            contact: ContactPatch {
                display_name: Some(display_name.into()),
                kind: "lead".into(),
                ..ContactPatch::default()
            },
        },
    )
    .expect("create contact")
}

/// Send one JSON-RPC request and return the whole response envelope.
fn request(server: &Server, method: &str, params: Value) -> Value {
    server
        .handle_message(json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
        .expect("a request always gets a response")
}

/// Call a tool and return the tool result object (`isError` included).
fn call(server: &Server, name: &str, arguments: Value) -> Value {
    let response = request(
        server,
        "tools/call",
        json!({"name": name, "arguments": arguments}),
    );
    assert!(
        response.get("error").is_none(),
        "unexpected JSON-RPC error: {response}"
    );
    response["result"].clone()
}

/// The wire JSON a successful tool call returned.
fn ok(server: &Server, name: &str, arguments: Value) -> Value {
    let result = call(server, name, arguments);
    assert_eq!(result["isError"], json!(false), "tool failed: {result}");
    result["structuredContent"]["result"].clone()
}

/// The stable error kind of a failed tool call.
fn kind(server: &Server, name: &str, arguments: Value) -> String {
    let result = call(server, name, arguments);
    assert_eq!(result["isError"], json!(true), "tool succeeded: {result}");
    result["structuredContent"]["error"]["kind"]
        .as_str()
        .expect("an error kind")
        .to_owned()
}

fn tool_names(server: &Server) -> Vec<String> {
    request(server, "tools/list", json!({}))["result"]["tools"]
        .as_array()
        .expect("a tool list")
        .iter()
        .map(|tool| tool["name"].as_str().expect("a tool name").to_owned())
        .collect()
}

fn handshake(server: &Server, client: &str) -> Value {
    request(
        server,
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": client, "version": "1.0"},
        }),
    )
}

// ---------------------------------------------------------------------------
// Handshake and tool listing
// ---------------------------------------------------------------------------

#[test]
fn initialize_reports_the_product_and_local_api_versions() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let server = server(&temp, storage, Mode::ReadOnly);

    let result = handshake(&server, "Claude Desktop")["result"].clone();

    assert_eq!(result["protocolVersion"], json!("2025-06-18"));
    assert_eq!(result["serverInfo"]["name"], json!("contractorcrm-mcp"));
    assert_eq!(
        result["_meta"]["productVersion"],
        json!(env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(result["_meta"]["localApiVersion"], json!(1));
    assert_eq!(result["_meta"]["mode"], json!("read_only"));
    assert_eq!(server.client_name(), "Claude Desktop");
}

#[test]
fn an_unsupported_protocol_revision_falls_back_to_ours() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let server = server(&temp, storage, Mode::ReadWrite);

    let result = request(
        &server,
        "initialize",
        json!({"protocolVersion": "1999-01-01", "clientInfo": {"name": "Old client"}}),
    )["result"]
        .clone();

    assert_eq!(result["protocolVersion"], json!("2025-06-18"));
    assert_eq!(result["_meta"]["mode"], json!("read_write"));
}

#[test]
fn read_only_mode_lists_no_write_tools() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let read_only = server(&temp, storage, Mode::ReadOnly);
    let names = tool_names(&read_only);

    assert!(names.contains(&"search_records".to_owned()));
    assert!(names.contains(&"propose_update".to_owned()));
    assert!(names.contains(&"preview_context".to_owned()));
    for write_tool in [
        "create_contact",
        "update_contact",
        "apply_proposal",
        "undo_proposal",
        "link_job",
    ] {
        assert!(
            !names.contains(&write_tool.to_owned()),
            "{write_tool} must not be listed read-only"
        );
    }

    let temp2 = tempfile::tempdir().expect("temp dir");
    let storage2 = open_storage(&temp2);
    let read_write = server(&temp2, storage2, Mode::ReadWrite);
    let write_names = tool_names(&read_write);
    assert!(write_names.contains(&"create_contact".to_owned()));
    assert!(write_names.len() > names.len());
}

#[test]
fn v1_omits_the_archive_csv_and_backup_tools() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let server = server(&temp, storage, Mode::ReadWrite);
    let names = tool_names(&server);

    for omitted in [
        "export_archive",
        "import_archive",
        "import_contacts",
        "export_contacts_csv",
        "backup_database",
        "restore_database",
        "add_attachment",
    ] {
        assert!(
            !names.contains(&omitted.to_owned()),
            "{omitted} is desktop-only in v1"
        );
    }
}

/// The whole advertised surface, so a tool can never be added or dropped
/// without docs/SLICE5_COVERAGE.md and docs/LOCAL_API.md being revisited.
#[test]
fn the_advertised_tool_surface_is_exactly_the_documented_one() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let server = server(&temp, storage, Mode::ReadWrite);
    let mut names = tool_names(&server);
    names.sort();

    let mut expected = ALL_TOOLS.map(str::to_owned).to_vec();
    expected.sort();
    assert_eq!(names, expected, "the MCP tool surface changed");

    // Every tool but the MCP-only context preview is a published v1 command.
    let schema: Value = serde_json::from_str(LOCAL_API_SCHEMA).expect("valid local API schema");
    let commands = schema["commands"]
        .as_array()
        .expect("published commands")
        .iter()
        .map(|command| command["name"].as_str().expect("a name").to_owned())
        .collect::<Vec<_>>();
    for name in &names {
        assert!(
            name == "preview_context" || commands.contains(name),
            "{name} is not a published v1 command"
        );
    }
}

// ---------------------------------------------------------------------------
// Mode enforcement and error mapping
// ---------------------------------------------------------------------------

#[test]
fn a_write_tool_on_a_read_only_connection_is_refused_by_name() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let server = server(&temp, storage, Mode::ReadOnly);

    let result = call(
        &server,
        "create_contact",
        json!({"contact": {"displayName": "Dana Ruiz", "kind": "client"}}),
    );

    assert_eq!(result["isError"], json!(true));
    let error = &result["structuredContent"]["error"];
    assert_eq!(error["kind"], json!("read_only"));
    assert_eq!(error["command"], json!("create_contact"));
}

#[test]
fn an_unknown_tool_is_a_json_rpc_error() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let server = server(&temp, storage, Mode::ReadWrite);

    let response = request(&server, "tools/call", json!({"name": "delete_everything"}));

    assert_eq!(response["error"]["code"], json!(-32602));
    assert!(response["error"]["message"]
        .as_str()
        .expect("a message")
        .contains("unknown tool"));
}

#[test]
fn an_unknown_method_is_a_json_rpc_error_and_notifications_get_no_reply() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let server = server(&temp, storage, Mode::ReadOnly);

    let response = request(&server, "resources/list", json!({}));
    assert_eq!(response["error"]["code"], json!(-32601));

    assert!(server
        .handle_message(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
        .is_none());
}

#[test]
fn malformed_arguments_map_to_invalid_input() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let server = server(&temp, storage, Mode::ReadOnly);

    assert_eq!(kind(&server, "get_contact", json!({})), "invalid_input");
    assert_eq!(
        kind(&server, "get_contact", json!({"contactId": 7})),
        "invalid_input"
    );
    assert_eq!(
        kind(
            &server,
            "search_records",
            json!({"query": "fence", "surprise": true})
        ),
        "invalid_input"
    );
}

#[test]
fn a_missing_record_maps_to_not_found() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let server = server(&temp, storage, Mode::ReadOnly);

    let result = call(&server, "get_contact", json!({"contactId": "nope"}));
    let error = &result["structuredContent"]["error"];
    assert_eq!(error["kind"], json!("not_found"));
    assert_eq!(error["recordId"], json!("nope"));
}

#[test]
fn a_stale_expected_version_surfaces_the_version_conflict_payload() {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Ruiz");
    let server = server(&temp, storage, Mode::ReadWrite);

    let result = call(
        &server,
        "update_contact",
        json!({
            "contactId": contact.id,
            "expectedVersion": contact.version + 5,
            "patch": {"displayName": "Dana Ruiz", "kind": "lead"},
        }),
    );

    let error = &result["structuredContent"]["error"];
    assert_eq!(error["kind"], json!("version_conflict"));
    assert_eq!(error["resource"], json!("contact"));
    assert_eq!(error["recordId"], json!(contact.id));
    assert_eq!(error["expectedVersion"], json!(contact.version + 5));
    assert_eq!(error["currentVersion"], json!(contact.version));
}

#[test]
fn an_unknown_draft_surfaces_proposal_expired() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let server = server(&temp, storage, Mode::ReadWrite);

    let result = call(&server, "apply_proposal", json!({"proposalId": "gone"}));
    let error = &result["structuredContent"]["error"];
    assert_eq!(error["kind"], json!("proposal_expired"));
    assert_eq!(error["proposalId"], json!("gone"));
}

// ---------------------------------------------------------------------------
// Bounded reads
// ---------------------------------------------------------------------------

#[test]
fn search_never_returns_more_than_fifty_rows() {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut storage = open_storage(&temp);
    for index in 0..60 {
        make_contact(&mut storage, &format!("Fence Crew {index}"));
    }
    let server = server(&temp, storage, Mode::ReadOnly);

    let results = ok(
        &server,
        "search_records",
        json!({"query": "Fence", "limit": 500}),
    );

    assert_eq!(results.as_array().expect("results").len(), 50);
}

#[test]
fn a_timeline_is_capped_and_its_bodies_truncated() {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Ruiz");
    let long_body = "note ".repeat(400);
    for index in 0..205 {
        application::log_activity(
            &mut storage,
            LogActivityRequest {
                actor: Actor::User,
                parent_type: "contact".into(),
                parent_id: contact.id.clone(),
                activity: ActivityPatch {
                    kind: "note".into(),
                    direction: None,
                    occurred_at: None,
                    summary: format!("Touch {index}"),
                    body: Some(long_body.clone()),
                },
            },
        )
        .expect("log activity");
    }
    let server = server(&temp, storage, Mode::ReadOnly);

    let entries = ok(
        &server,
        "get_timeline",
        json!({"parentType": "contact", "parentId": contact.id}),
    );
    let entries = entries.as_array().expect("entries");
    assert_eq!(entries.len(), 200, "the entry cap applies without a limit");
    let body = entries[0]["body"].as_str().expect("a body");
    assert!(body.ends_with("… (truncated)"));
    assert_eq!(
        body.chars().count(),
        MAX_TIMELINE_BODY_CHARS + "… (truncated)".chars().count()
    );

    // Asking for full bodies is explicit, and over-large limits are refused.
    let full = ok(
        &server,
        "get_timeline",
        json!({"parentType": "contact", "parentId": contact.id, "limit": 1, "fullBodies": true}),
    );
    // Stored bodies are trimmed on write, so compare against the trimmed text.
    assert_eq!(full[0]["body"].as_str().expect("a body"), long_body.trim());
    assert_eq!(
        kind(
            &server,
            "get_timeline",
            json!({"parentType": "contact", "parentId": contact.id, "limit": 5000})
        ),
        "invalid_input"
    );
}

#[test]
fn attachment_reads_carry_no_file_contents_or_internal_paths() {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Ruiz");
    let store = AttachmentStore::open_in_app_data(temp.path());
    let source = temp.path().join("scope.txt");
    std::fs::write(&source, b"secret scope of work").expect("write source file");
    attachments::add_attachment(
        &mut storage,
        &store,
        AddAttachmentRequest {
            actor: Actor::User,
            parent_type: AttachmentParentType::Contact,
            parent_id: contact.id.clone(),
            source_path: source.to_string_lossy().into_owned(),
        },
    )
    .expect("add attachment");
    let server = server(&temp, storage, Mode::ReadOnly);

    let listed = ok(
        &server,
        "list_attachments",
        json!({"parentType": "contact", "parentId": contact.id}),
    );
    let attachment = &listed.as_array().expect("attachments")[0];
    assert_eq!(attachment["fileName"], json!("scope.txt"));
    for leaked in ["relativePath", "relative_path", "body", "contents", "bytes"] {
        assert!(
            attachment.get(leaked).is_none(),
            "{leaked} must never be returned"
        );
    }
    assert!(!listed.to_string().contains("secret scope of work"));

    let location = ok(
        &server,
        "attachment_path",
        json!({"attachmentId": attachment["id"]}),
    );
    assert_eq!(location["exists"], json!(true));
}

// ---------------------------------------------------------------------------
// AI-backed tools and the context preview
// ---------------------------------------------------------------------------

#[test]
fn preview_context_shows_what_would_be_sent_without_calling_the_provider() {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Ruiz");
    application::log_activity(
        &mut storage,
        LogActivityRequest {
            actor: Actor::User,
            parent_type: "contact".into(),
            parent_id: contact.id.clone(),
            activity: ActivityPatch {
                kind: "call".into(),
                direction: None,
                occurred_at: None,
                summary: "Walked the back fence line".into(),
                body: None,
            },
        },
    )
    .expect("log activity");
    // The assistant stays OFF: a preview must never need a provider.
    let provider = Arc::new(CannedProvider::new("unused"));
    let server = server(&temp, storage, Mode::ReadOnly).with_provider(provider.clone());

    let preview = ok(
        &server,
        "preview_context",
        json!({
            "tool": "summarize_history",
            "arguments": {"parentType": "contact", "parentId": contact.id},
        }),
    );

    assert_eq!(preview["purpose"], json!("summarize_history"));
    let text = preview["contextText"].as_str().expect("context text");
    assert!(text.contains("Dana Ruiz"));
    assert!(text.contains("Walked the back fence line"));
    assert_eq!(
        preview["includedRecordRefs"][0]["entityId"],
        json!(contact.id)
    );
    assert_eq!(
        preview["includedRecordRefs"]
            .as_array()
            .expect("refs")
            .len(),
        1
    );
    assert_eq!(provider.call_count(), 0, "a preview sends nothing");
}

#[test]
fn summarize_history_calls_the_provider_only_when_the_tool_is_invoked() {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Ruiz");
    turn_the_assistant_on(&mut storage);
    let provider = Arc::new(CannedProvider::new(
        "Summary: Quiet since the walk-through.\nNext actions:\n- Call Dana",
    ));
    let server = server(&temp, storage, Mode::ReadOnly).with_provider(provider.clone());

    // Listing and reading never touch the provider.
    let _ = tool_names(&server);
    let _ = ok(&server, "get_contact", json!({"contactId": contact.id}));
    assert_eq!(provider.call_count(), 0);

    let summary = ok(
        &server,
        "summarize_history",
        json!({"parentType": "contact", "parentId": contact.id}),
    );

    assert_eq!(provider.call_count(), 1);
    assert_eq!(summary["model"], json!("canned-model"));
    assert_eq!(summary["suggestedNextActions"], json!(["Call Dana"]));
    assert_eq!(
        summary["includedRecordRefs"][0]["entityId"],
        json!(contact.id)
    );
}

#[test]
fn the_assistant_being_off_is_provider_unavailable() {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Ruiz");
    let server = server(&temp, storage, Mode::ReadOnly);

    assert_eq!(
        kind(
            &server,
            "summarize_history",
            json!({"parentType": "contact", "parentId": contact.id})
        ),
        "provider_unavailable"
    );
}

// ---------------------------------------------------------------------------
// Propose → apply → undo, and the audit trail
// ---------------------------------------------------------------------------

#[test]
fn a_draft_can_be_proposed_applied_and_undone_over_mcp() {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut storage = open_storage(&temp);
    turn_the_assistant_on(&mut storage);
    let provider = Arc::new(CannedProvider::new(
        r#"{"displayName":"Dana Ruiz","kind":"client"}"#,
    ));
    let server = server(&temp, storage, Mode::ReadWrite).with_provider(provider);
    handshake(&server, "Claude Desktop");

    let proposal = ok(
        &server,
        "propose_record",
        json!({"kind": "contact", "description": "New client Dana Ruiz"}),
    );
    assert_eq!(proposal["kind"], json!("create_contact"));

    let applied = ok(
        &server,
        "apply_proposal",
        json!({"proposalId": proposal["id"]}),
    );
    assert_eq!(applied["created"], json!(true));
    assert_eq!(applied["entityType"], json!("contact"));

    let contact = ok(
        &server,
        "get_contact",
        json!({"contactId": applied["entityId"]}),
    );
    assert_eq!(contact["displayName"], json!("Dana Ruiz"));

    let undone = ok(
        &server,
        "undo_proposal",
        json!({"undoToken": applied["undoToken"]}),
    );
    assert_eq!(undone["action"], json!("archived"));

    // Single use: the same draft cannot be applied twice.
    let result = call(
        &server,
        "apply_proposal",
        json!({"proposalId": proposal["id"]}),
    );
    assert_eq!(
        result["structuredContent"]["error"]["kind"],
        json!("proposal_expired")
    );
}

#[test]
fn writes_are_logged_against_the_agent_actor_and_the_client_name() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let server = server(&temp, storage, Mode::ReadWrite);
    handshake(&server, "Claude Desktop");

    let contact = ok(
        &server,
        "create_contact",
        json!({"contact": {"displayName": "Dana Ruiz", "kind": "client"}}),
    );
    let contact_id = contact["id"].as_str().expect("a contact id").to_owned();

    // Read the log through a second connection to the same database file.
    let audit = open_storage(&temp);
    let mut statement = audit
        .connection()
        .prepare("SELECT actor, summary FROM command_log WHERE entity_id = ?1 ORDER BY created_at")
        .expect("prepare");
    let rows = statement
        .query_map([&contact_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("rows");

    assert!(
        rows.len() >= 2,
        "record row plus the client attribution row"
    );
    assert!(
        rows.iter().all(|(actor, _)| actor == "agent"),
        "every MCP write is the agent actor: {rows:?}"
    );
    assert!(
        rows.iter()
            .any(|(_, summary)| summary.contains("create_contact")
                && summary.contains("Claude Desktop")),
        "the client name is recorded: {rows:?}"
    );
}

// ---------------------------------------------------------------------------
// The rest of the read surface
// ---------------------------------------------------------------------------

#[test]
fn the_read_tools_answer_for_every_record_and_metadata_surface() {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Ruiz");
    let company = application::create_company(
        &mut storage,
        CreateCompanyRequest {
            actor: Actor::User,
            company: CompanyPatch {
                name: "Ridgeline Fence Co".into(),
                kind: "client".into(),
                ..CompanyPatch::default()
            },
        },
    )
    .expect("create company");
    let opportunity = application::create_opportunity(
        &mut storage,
        CreateOpportunityRequest {
            actor: Actor::User,
            stage_id: None,
            opportunity: OpportunityPatch {
                name: "Back lot fence".into(),
                contact_id: Some(contact.id.clone()),
                currency_code: "USD".into(),
                ..OpportunityPatch::default()
            },
        },
    )
    .expect("create opportunity");
    application::create_task(
        &mut storage,
        CreateTaskRequest {
            actor: Actor::User,
            task: TaskPatch {
                title: "Call Dana back".into(),
                parent_type: Some("contact".into()),
                parent_id: Some(contact.id.clone()),
                ..TaskPatch::default()
            },
        },
    )
    .expect("create task");
    let server = server(&temp, storage, Mode::ReadOnly);

    let companies = ok(&server, "list_companies", json!({}));
    assert_eq!(companies[0]["id"], json!(company.id));
    assert_eq!(
        ok(&server, "get_company", json!({"companyId": company.id}))["name"],
        json!("Ridgeline Fence Co")
    );

    let opportunities = ok(&server, "list_opportunities", json!({}));
    assert_eq!(opportunities[0]["id"], json!(opportunity.id));
    assert_eq!(
        ok(
            &server,
            "get_opportunity",
            json!({"opportunityId": opportunity.id})
        )["name"],
        json!("Back lot fence")
    );

    let tasks = ok(&server, "list_tasks", json!({"status": "open"}));
    assert_eq!(tasks[0]["title"], json!("Call Dana back"));
    assert_eq!(
        ok(&server, "list_tasks", json!({"overdueOnly": true}))
            .as_array()
            .expect("tasks")
            .len(),
        0,
        "a task with no due date is never overdue"
    );

    // Deterministic flags, saved views, tags, custom fields, and the stored
    // follow-up wordings all answer over the same read connection.
    assert!(ok(&server, "get_attention_flags", json!({})).is_array());
    assert!(ok(
        &server,
        "list_saved_views",
        json!({"entityType": "contact"})
    )
    .is_array());
    assert!(ok(&server, "list_tags", json!({})).is_array());
    assert!(ok(
        &server,
        "list_custom_field_defs",
        json!({"entityType": "contact"})
    )
    .is_array());

    let metadata = ok(
        &server,
        "get_record_metadata",
        json!({"entityType": "contact", "recordId": contact.id}),
    );
    assert!(metadata["tagIds"].is_array());
    assert!(metadata["values"].is_array());

    let templates = ok(&server, "get_followup_templates", json!({}));
    assert_eq!(templates["version"], json!(1));
    assert!(!templates["templates"]
        .as_array()
        .expect("templates")
        .is_empty());
}

#[test]
fn list_tools_take_a_limit_and_refuse_an_unusable_one() {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut storage = open_storage(&temp);
    for index in 0..5 {
        make_contact(&mut storage, &format!("Crew {index}"));
    }
    let server = server(&temp, storage, Mode::ReadOnly);

    assert_eq!(
        ok(&server, "list_contacts", json!({"limit": 2}))
            .as_array()
            .expect("contacts")
            .len(),
        2
    );
    assert_eq!(
        ok(&server, "list_contacts", json!({}))
            .as_array()
            .expect("contacts")
            .len(),
        5,
        "no limit returns everything active"
    );
    // Zero and over-cap limits are caller errors, never a silent clamp.
    assert_eq!(
        kind(&server, "list_contacts", json!({"limit": 0})),
        "invalid_input"
    );
    assert_eq!(
        kind(&server, "list_tasks", json!({"limit": MAX_LIST_LIMIT + 1})),
        "invalid_input"
    );
}

// ---------------------------------------------------------------------------
// The rest of the write surface
// ---------------------------------------------------------------------------

#[test]
fn every_write_tool_round_trips_through_the_ordinary_application_path() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let won_stage = application::list_stages(&storage)
        .expect("list stages")
        .into_iter()
        .find(|stage| stage.kind == StageKind::Won)
        .expect("a won stage is seeded");
    let server = server(&temp, storage, Mode::ReadWrite);
    handshake(&server, "Claude Desktop");

    let company = ok(
        &server,
        "create_company",
        json!({"company": {"name": "Ridgeline Fence Co", "kind": "client"}}),
    );
    let company_updated = ok(
        &server,
        "update_company",
        json!({
            "companyId": company["id"],
            "expectedVersion": company["version"],
            "patch": {"name": "Ridgeline Fence", "kind": "client"},
        }),
    );
    assert_eq!(company_updated["name"], json!("Ridgeline Fence"));

    let opportunity = ok(
        &server,
        "create_opportunity",
        json!({
            "opportunity": {
                "name": "Back lot fence",
                "companyId": company["id"],
                "currencyCode": "USD",
                "valueMinor": 450_000,
            },
        }),
    );
    let opportunity = ok(
        &server,
        "update_opportunity",
        json!({
            "opportunityId": opportunity["id"],
            "expectedVersion": opportunity["version"],
            "patch": {
                "name": "Back lot fence — 240 lf",
                "companyId": company["id"],
                "currencyCode": "USD",
                "valueMinor": 480_000,
            },
        }),
    );
    assert_eq!(opportunity["value"]["valueMinor"], json!(480_000));

    let activity = ok(
        &server,
        "log_activity",
        json!({
            "parentType": "opportunity",
            "parentId": opportunity["id"],
            "activity": {"kind": "call", "summary": "Walked the line with the owner"},
        }),
    );
    assert_eq!(activity["summary"], json!("Walked the line with the owner"));

    let task = ok(
        &server,
        "create_task",
        json!({
            "task": {
                "title": "Send the fence quote",
                "parentType": "opportunity",
                "parentId": opportunity["id"],
            },
        }),
    );
    let completed = ok(
        &server,
        "complete_task",
        json!({"taskId": task["id"], "expectedVersion": task["version"]}),
    );
    assert_eq!(completed["status"], json!("done"));

    let quoted = ok(
        &server,
        "link_quote",
        json!({
            "opportunityId": opportunity["id"],
            "expectedVersion": opportunity["version"],
            "quoteRef": {"tool": "contractorquote", "externalId": "q-42", "label": "Quote 42"},
        }),
    );
    assert_eq!(quoted["quoteRef"]["externalId"], json!("q-42"));

    // A job hand-off is only allowed once the opportunity is won.
    let won = ok(
        &server,
        "move_opportunity_stage",
        json!({
            "opportunityId": opportunity["id"],
            "toStageId": won_stage.id,
            "expectedVersion": quoted["version"],
        }),
    );
    assert_eq!(won["stageId"], json!(won_stage.id));
    let linked = ok(
        &server,
        "link_job",
        json!({
            "opportunityId": opportunity["id"],
            "expectedVersion": won["version"],
            "jobRef": {"tool": "contractorproject", "externalId": "job-7"},
        }),
    );
    assert_eq!(linked["jobRef"]["externalId"], json!("job-7"));
}

#[test]
fn every_version_checked_write_reports_the_conflict_over_mcp() {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Ruiz");
    let company = application::create_company(
        &mut storage,
        CreateCompanyRequest {
            actor: Actor::User,
            company: CompanyPatch {
                name: "Ridgeline Fence Co".into(),
                kind: "client".into(),
                ..CompanyPatch::default()
            },
        },
    )
    .expect("create company");
    let opportunity = application::create_opportunity(
        &mut storage,
        CreateOpportunityRequest {
            actor: Actor::User,
            stage_id: None,
            opportunity: OpportunityPatch {
                name: "Back lot fence".into(),
                contact_id: Some(contact.id.clone()),
                currency_code: "USD".into(),
                ..OpportunityPatch::default()
            },
        },
    )
    .expect("create opportunity");
    let task = application::create_task(
        &mut storage,
        CreateTaskRequest {
            actor: Actor::User,
            task: TaskPatch {
                title: "Call Dana back".into(),
                ..TaskPatch::default()
            },
        },
    )
    .expect("create task");
    let won_stage = application::list_stages(&storage)
        .expect("list stages")
        .into_iter()
        .find(|stage| stage.kind == StageKind::Won)
        .expect("a won stage is seeded");
    let stale = 99;
    let server = server(&temp, storage, Mode::ReadWrite);

    for (tool, arguments, resource) in [
        (
            "update_company",
            json!({"companyId": company.id, "expectedVersion": stale,
                   "patch": {"name": "Ridgeline Fence", "kind": "client"}}),
            "company",
        ),
        (
            "update_opportunity",
            json!({"opportunityId": opportunity.id, "expectedVersion": stale,
                   "patch": {"name": "Back lot fence", "contactId": contact.id,
                             "currencyCode": "USD"}}),
            "opportunity",
        ),
        (
            "move_opportunity_stage",
            json!({"opportunityId": opportunity.id, "toStageId": won_stage.id,
                   "expectedVersion": stale}),
            "opportunity",
        ),
        (
            "complete_task",
            json!({"taskId": task.id, "expectedVersion": stale}),
            "task",
        ),
        (
            "link_quote",
            json!({"opportunityId": opportunity.id, "expectedVersion": stale,
                   "quoteRef": {"tool": "contractorquote", "externalId": "q-42"}}),
            "opportunity",
        ),
        (
            "link_job",
            json!({"opportunityId": opportunity.id, "expectedVersion": stale,
                   "jobRef": {"tool": "contractorproject", "externalId": "job-7"}}),
            "opportunity",
        ),
    ] {
        let result = call(&server, tool, arguments);
        let error = &result["structuredContent"]["error"];
        assert_eq!(error["kind"], json!("version_conflict"), "{tool}: {result}");
        assert_eq!(error["resource"], json!(resource), "{tool}");
        assert_eq!(error["expectedVersion"], json!(stale), "{tool}");
        assert!(
            error["currentVersion"].as_i64().expect("a version") < stale,
            "{tool} must report the current version"
        );
    }
}

#[test]
fn record_rules_and_the_lost_reason_rule_carry_their_own_error_kinds() {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Ruiz");
    let opportunity = application::create_opportunity(
        &mut storage,
        CreateOpportunityRequest {
            actor: Actor::User,
            stage_id: None,
            opportunity: OpportunityPatch {
                name: "Back lot fence".into(),
                contact_id: Some(contact.id.clone()),
                currency_code: "USD".into(),
                ..OpportunityPatch::default()
            },
        },
    )
    .expect("create opportunity");
    let lost_stage = application::list_stages(&storage)
        .expect("list stages")
        .into_iter()
        .find(|stage| stage.kind == StageKind::Lost)
        .expect("a lost stage is seeded");
    let server = server(&temp, storage, Mode::ReadWrite);

    // A record rule the desktop enforces is the same rule here, with its code
    // and field path intact.
    let rejected = call(
        &server,
        "create_contact",
        json!({
            "contact": {
                "displayName": "Sam Boone",
                "kind": "client",
                "channels": [
                    {"kind": "phone", "value": "555-0101", "preferred": true},
                    {"kind": "phone", "value": "555-0102", "preferred": true},
                ],
            },
        }),
    );
    let error = &rejected["structuredContent"]["error"];
    assert_eq!(error["kind"], json!("validation_failed"));
    assert_eq!(error["code"], json!("duplicate_preferred_channel"));
    assert_eq!(error["field"], json!("channels[1].preferred"));

    let lost = call(
        &server,
        "move_opportunity_stage",
        json!({
            "opportunityId": opportunity.id,
            "toStageId": lost_stage.id,
            "expectedVersion": opportunity.version,
        }),
    );
    let error = &lost["structuredContent"]["error"];
    assert_eq!(error["kind"], json!("missing_lost_reason"));
    assert_eq!(error["recordId"], json!(opportunity.id));
}

/// An agent has to be able to discover stage and lost-reason ids, or
/// `move_opportunity_stage` is unusable without a human reading them out.
#[test]
fn an_agent_can_discover_stage_and_lost_reason_ids_and_move_work_with_them() {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Ruiz");
    let opportunity = application::create_opportunity(
        &mut storage,
        CreateOpportunityRequest {
            actor: Actor::User,
            stage_id: None,
            opportunity: OpportunityPatch {
                name: "Back yard fence".into(),
                contact_id: Some(contact.id),
                value_minor: 450_000,
                currency_code: "USD".into(),
                ..OpportunityPatch::default()
            },
        },
    )
    .expect("create opportunity");
    let server = server(&temp, storage, Mode::ReadWrite);

    let stages = ok(&server, "list_stages", json!({}));
    let stages = stages.as_array().expect("stages");
    assert!(stages.len() >= 3, "a seeded pipeline has open/won/lost");
    let lost_stage_id = stages
        .iter()
        .find(|stage| stage["kind"] == json!("lost"))
        .expect("a lost stage")["id"]
        .clone();

    let reasons = ok(&server, "list_lost_reasons", json!({}));
    let lost_reason_id = reasons.as_array().expect("lost reasons")[0]["id"].clone();

    // Both ids came from the tools, and nothing else was needed.
    let moved = ok(
        &server,
        "move_opportunity_stage",
        json!({
            "opportunityId": opportunity.id,
            "toStageId": lost_stage_id,
            "lostReasonId": lost_reason_id,
            "expectedVersion": opportunity.version,
        }),
    );
    assert_eq!(moved["stageId"], lost_stage_id);
}

// ---------------------------------------------------------------------------
// The remaining AI-backed tools
// ---------------------------------------------------------------------------

#[test]
fn explain_attention_flag_answers_the_flag_get_attention_flags_returned() {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut storage = open_storage(&temp);
    let stale = make_contact(&mut storage, "Stale Sam");
    application::log_activity(
        &mut storage,
        LogActivityRequest {
            actor: Actor::User,
            parent_type: "contact".into(),
            parent_id: stale.id.clone(),
            activity: ActivityPatch {
                kind: "call".into(),
                direction: Some("outbound".into()),
                occurred_at: Some(
                    (Utc::now() - Duration::days(40)).to_rfc3339_opts(SecondsFormat::Millis, true),
                ),
                summary: "Phone call".into(),
                body: None,
            },
        },
    )
    .expect("log activity");
    turn_the_assistant_on(&mut storage);
    let provider = Arc::new(CannedProvider::new(
        "Sam has gone quiet for 40 days. Call him this week.",
    ));
    let server = server(&temp, storage, Mode::ReadOnly).with_provider(provider.clone());

    let flags = ok(&server, "get_attention_flags", json!({}));
    let flag_id = flags[0]["id"].as_str().expect("a flag id").to_owned();
    assert!(flag_id.starts_with("stale_lead:"), "{flag_id}");

    // The preview names the flagged lead and sends nothing.
    let preview = ok(
        &server,
        "preview_context",
        json!({"tool": "explain_attention_flag", "arguments": {"flagId": flag_id}}),
    );
    assert_eq!(preview["purpose"], json!("explain_attention_flag"));
    assert_eq!(
        preview["includedRecordRefs"][0]["entityId"],
        json!(stale.id)
    );
    assert_eq!(provider.call_count(), 0);

    let explanation = ok(
        &server,
        "explain_attention_flag",
        json!({"flagId": flag_id}),
    );
    assert_eq!(provider.call_count(), 1);
    assert_eq!(explanation["flagId"], json!(flag_id));
    assert_eq!(explanation["explanation"]["model"], json!("canned-model"));

    // A flag that is no longer current is not_found.
    assert_eq!(
        kind(
            &server,
            "explain_attention_flag",
            json!({"flagId": "stale_lead:gone"})
        ),
        "not_found"
    );
}

#[test]
fn propose_followup_drafts_from_a_template_and_applies_as_a_task() {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Ruiz");
    // The assistant stays OFF: drafting must still work from the template.
    let server = server(&temp, storage, Mode::ReadWrite);
    handshake(&server, "Claude Desktop");

    let draft = ok(
        &server,
        "propose_followup",
        json!({"parentType": "contact", "parentId": contact.id}),
    );
    assert_eq!(draft["usedProvider"], json!(false));
    assert!(!draft["draftText"].as_str().expect("draft text").is_empty());
    assert_eq!(
        draft["includedRecordRefs"].as_array().expect("refs").len(),
        0,
        "template-only drafting sends nothing"
    );
    let proposal = &draft["proposal"];
    assert_eq!(proposal["kind"], json!("create_followup_task"));

    let applied = ok(
        &server,
        "apply_proposal",
        json!({"proposalId": proposal["id"]}),
    );
    assert_eq!(applied["entityType"], json!("task"));
    let tasks = ok(
        &server,
        "list_tasks",
        json!({"parentType": "contact", "parentId": contact.id}),
    );
    assert_eq!(tasks.as_array().expect("tasks").len(), 1);

    // An unknown template id is a caller error, not a silent fallback.
    assert_eq!(
        kind(
            &server,
            "propose_followup",
            json!({"parentType": "contact", "parentId": contact.id, "templateId": "nope"})
        ),
        "not_found"
    );
}

/// The follow-up preview has to describe the follow-up call, not the summary
/// call: it takes the same arguments and projects the same text.
#[test]
fn the_followup_preview_matches_what_propose_followup_actually_sends() {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Ruiz");
    application::log_activity(
        &mut storage,
        LogActivityRequest {
            actor: Actor::User,
            parent_type: "contact".into(),
            parent_id: contact.id.clone(),
            activity: ActivityPatch {
                kind: "call".into(),
                direction: None,
                occurred_at: None,
                summary: "Walked the back fence line".into(),
                body: None,
            },
        },
    )
    .expect("log activity");
    let provider = Arc::new(CannedProvider::new("Checking in on that fence quote."));
    let server = server(&temp, storage, Mode::ReadOnly).with_provider(provider.clone());
    let arguments = json!({
        "parentType": "contact",
        "parentId": contact.id,
        "objective": "chase the proposal",
        "templateId": "proposal_chaser",
    });

    // The arguments a caller would pass to the tool are accepted as-is.
    let preview = ok(
        &server,
        "preview_context",
        json!({"tool": "propose_followup", "arguments": arguments}),
    );
    assert_eq!(preview["purpose"], json!("propose_followup"));
    assert_eq!(
        preview["includedRecordRefs"][0]["entityId"],
        json!(contact.id)
    );
    assert_eq!(provider.call_count(), 0, "a preview sends nothing");

    // A window argument is accepted and ignored: drafting has one window.
    let mut windowed = arguments.clone();
    windowed["window"] = json!(1);
    let windowed_preview = ok(
        &server,
        "preview_context",
        json!({"tool": "propose_followup", "arguments": windowed}),
    );
    assert_eq!(windowed_preview["contextText"], preview["contextText"]);

    // And the projection is exactly what the real call carries.
    ok(&server, "propose_followup", arguments);
    let sent = provider.seen.lock().expect("canned provider mutex")[0].clone();
    assert_eq!(sent.purpose, "propose_followup");
    assert_eq!(
        preview["contextText"].as_str().expect("context text"),
        sent.context_text.as_deref().expect("context was sent")
    );
}

#[test]
fn preview_context_covers_propose_update_and_refuses_an_unpreviewable_tool() {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Ruiz");
    let provider = Arc::new(CannedProvider::new("unused"));
    let server = server(&temp, storage, Mode::ReadOnly).with_provider(provider.clone());

    let preview = ok(
        &server,
        "preview_context",
        json!({
            "tool": "propose_update",
            "arguments": {
                "entityType": "contact",
                "entityId": contact.id,
                "expectedVersion": contact.version,
            },
        }),
    );
    assert_eq!(preview["purpose"], json!("propose_update"));
    assert!(preview["contextText"]
        .as_str()
        .expect("context text")
        .contains("Dana Ruiz"));
    assert_eq!(
        preview["includedRecordRefs"][0]["entityId"],
        json!(contact.id)
    );
    assert_eq!(provider.call_count(), 0, "a preview sends nothing");

    // A stale version is caught before any model would be asked.
    assert_eq!(
        kind(
            &server,
            "preview_context",
            json!({
                "tool": "propose_update",
                "arguments": {
                    "entityType": "contact",
                    "entityId": contact.id,
                    "expectedVersion": contact.version + 5,
                },
            })
        ),
        "version_conflict"
    );
    assert_eq!(
        kind(
            &server,
            "preview_context",
            json!({"tool": "list_contacts", "arguments": {}})
        ),
        "invalid_input"
    );
}

// ---------------------------------------------------------------------------
// The real binary
// ---------------------------------------------------------------------------

#[test]
fn the_shipped_binary_serves_a_handshake_and_a_read_over_stdio() {
    let temp = tempfile::tempdir().expect("temp dir");
    let database = {
        let mut storage = open_storage(&temp);
        make_contact(&mut storage, "Dana Ruiz");
        storage.database_path().to_path_buf()
    };

    let mut child = Command::new(env!("CARGO_BIN_EXE_contractorcrm-mcp"))
        .arg("--database")
        .arg(&database)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn the helper");

    {
        let stdin = child.stdin.as_mut().expect("stdin");
        for message in [
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                   "params": {"protocolVersion": "2025-06-18", "clientInfo": {"name": "Test client"}}}),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}),
            json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
                   "params": {"name": "list_contacts", "arguments": {}}}),
        ] {
            writeln!(stdin, "{message}").expect("write a request");
        }
        stdin.flush().expect("flush");
    }
    // Closing stdin is the graceful shutdown.
    drop(child.stdin.take());

    let stdout = BufReader::new(child.stdout.take().expect("stdout"));
    let responses = stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(&line.expect("a line")).expect("valid JSON"))
        .collect::<Vec<_>>();
    let status = child.wait().expect("wait for the helper");

    assert!(status.success(), "the helper exited with {status}");
    assert_eq!(responses.len(), 3, "notifications get no reply");
    assert_eq!(
        responses[0]["result"]["serverInfo"]["name"],
        json!("contractorcrm-mcp")
    );
    assert_eq!(responses[0]["result"]["_meta"]["localApiVersion"], json!(1));

    let names = responses[1]["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .map(|tool| tool["name"].as_str().expect("a name").to_owned())
        .collect::<Vec<_>>();
    assert!(names.contains(&"list_contacts".to_owned()));
    assert!(
        !names.contains(&"create_contact".to_owned()),
        "the binary defaults to read-only"
    );

    let contacts = &responses[2]["result"]["structuredContent"]["result"];
    assert_eq!(contacts[0]["displayName"], json!("Dana Ruiz"));
}

// ---------------------------------------------------------------------------
// Opening a database (the helper must never rewrite a schema it wasn't given
// permission to touch)
// ---------------------------------------------------------------------------

/// Undo the newest migration so the file looks like one written by an older
/// build: drop what v11 created, put back what it removed, and forget it was
/// ever applied.
fn roll_back_the_newest_migration(database_path: &std::path::Path) {
    assert_eq!(
        contractorcrm_lib::storage::latest_migration_version(),
        11,
        "update this fixture when a migration is added"
    );
    let connection = rusqlite::Connection::open(database_path).expect("open the database");
    connection
        .execute_batch(
            "DROP INDEX tasks_parent_status_due;
             CREATE INDEX tasks_parent ON tasks(parent_type, parent_id);
             DELETE FROM schema_migrations WHERE version = 11;",
        )
        .expect("roll back migration 11");
}

fn stored_schema_version(database_path: &std::path::Path) -> i64 {
    let connection = rusqlite::Connection::open(database_path).expect("open the database");
    connection
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )
        .expect("read the schema version")
}

#[test]
fn a_read_only_helper_refuses_an_older_database_instead_of_migrating_it() {
    let temp = tempfile::tempdir().expect("temp dir");
    let database = {
        let storage = open_storage(&temp);
        storage.database_path().to_path_buf()
    };
    roll_back_the_newest_migration(&database);

    let error = Server::open(&database, Mode::ReadOnly)
        .err()
        .expect("an older file is refused");

    assert!(error.contains("schema v10"), "{error}");
    assert!(error.contains("desktop app"), "{error}");
    assert_eq!(
        stored_schema_version(&database),
        10,
        "a read-only connection must not migrate the user's database"
    );
}

#[test]
fn a_read_write_helper_may_still_migrate_an_older_database() {
    let temp = tempfile::tempdir().expect("temp dir");
    let database = {
        let storage = open_storage(&temp);
        storage.database_path().to_path_buf()
    };
    roll_back_the_newest_migration(&database);

    assert!(
        Server::open(&database, Mode::ReadWrite).is_ok(),
        "--read-write is an explicit write grant"
    );

    assert_eq!(stored_schema_version(&database), 11);
}

#[test]
fn a_foreign_sqlite_file_is_refused_rather_than_given_a_contractorcrm_schema() {
    let temp = tempfile::tempdir().expect("temp dir");
    let foreign = temp.path().join("someone-elses.sqlite3");
    {
        let connection = rusqlite::Connection::open(&foreign).expect("create a foreign database");
        connection
            .execute_batch("CREATE TABLE invoices (id TEXT PRIMARY KEY, total INTEGER);")
            .expect("seed the foreign schema");
    }

    for mode in [Mode::ReadOnly, Mode::ReadWrite] {
        let error = Server::open(&foreign, mode)
            .err()
            .expect("not a ContractorCRM database");
        assert!(
            error.contains("no ContractorCRM schema"),
            "{mode:?}: {error}"
        );
    }

    let connection = rusqlite::Connection::open(&foreign).expect("reopen the foreign database");
    let tables: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name != 'invoices'",
            [],
            |row| row.get(0),
        )
        .expect("count tables");
    assert_eq!(tables, 0, "the helper must not write a schema into it");
}

#[test]
fn the_binary_refuses_a_missing_database() {
    let temp = tempfile::tempdir().expect("temp dir");
    let missing = temp.path().join("not-here.sqlite3");

    let output = Command::new(env!("CARGO_BIN_EXE_contractorcrm-mcp"))
        .arg("--database")
        .arg(&missing)
        .output()
        .expect("run the helper");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("no ContractorCRM database"), "{stderr}");
}

// ---------------------------------------------------------------------------
// Hostile client probes (docs/THREAT_MODEL.md "MCP helper")
// ---------------------------------------------------------------------------

#[test]
fn an_oversized_stdio_message_is_refused_and_the_next_one_still_works() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let server = server(&temp, storage, Mode::ReadOnly);

    // One line the client never terminates until it is far past the cap, then
    // an ordinary request behind it.
    let mut input = Vec::new();
    input.extend(std::iter::repeat_n(b'x', MAX_MESSAGE_BYTES + 1024));
    input.extend_from_slice(b"\n");
    input.extend_from_slice(br#"{"jsonrpc":"2.0","id":7,"method":"ping","params":{}}"#);
    input.extend_from_slice(b"\n");

    let mut output = Vec::new();
    contractorcrm_lib::mcp::serve(&server, std::io::Cursor::new(input), &mut output)
        .expect("serve drains the oversized line");

    let responses = String::from_utf8(output).expect("utf-8 responses");
    let mut lines = responses.lines();
    let refusal: Value = serde_json::from_str(lines.next().expect("a refusal")).expect("json");
    assert_eq!(refusal["error"]["code"], json!(-32600));
    assert!(
        refusal["error"]["message"]
            .as_str()
            .expect("a message")
            .contains("larger than"),
        "{refusal}"
    );
    let pong: Value = serde_json::from_str(lines.next().expect("a second response")).expect("json");
    assert_eq!(pong["id"], json!(7), "the reader resynchronized: {pong}");
}

#[test]
fn hostile_tool_arguments_come_back_as_errors_not_panics() {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut storage = open_storage(&temp);
    let contact = make_contact(&mut storage, "Dana Ruiz");
    let server = server(&temp, storage, Mode::ReadWrite);

    // Traversal and absolute paths are record ids here, not paths: nothing
    // resolves, so they are plain not-found answers.
    assert_eq!(
        kind(
            &server,
            "attachment_path",
            json!({"attachmentId": "../../../etc/passwd"}),
        ),
        "not_found"
    );
    assert_eq!(
        kind(&server, "get_contact", json!({"contactId": "C:evil"})),
        "not_found"
    );
    // Oversized and out-of-range bounds are refused rather than clamped
    // silently or used to allocate.
    assert_eq!(
        kind(
            &server,
            "list_contacts",
            json!({"limit": MAX_LIST_LIMIT + 1}),
        ),
        "invalid_input"
    );
    assert_eq!(
        kind(
            &server,
            "get_timeline",
            json!({"parentType": "contact", "parentId": contact.id, "limit": 100_000}),
        ),
        "invalid_input"
    );
    // A SQL-shaped search string is treated as text: the FTS query is rebuilt
    // from alphanumeric words only.
    let results = ok(
        &server,
        "search_records",
        json!({"query": "'; DROP TABLE contacts; --"}),
    );
    assert!(results.as_array().expect("results").is_empty(), "{results}");
    let still_there = ok(&server, "list_contacts", json!({}));
    assert_eq!(still_there.as_array().expect("contacts").len(), 1);
    // Wrong types and unknown fields never reach the application layer.
    assert_eq!(
        kind(&server, "get_contact", json!({"contactId": 42})),
        "invalid_input"
    );
    assert_eq!(
        kind(
            &server,
            "list_contacts",
            json!({"includeArchived": "yes please"}),
        ),
        "invalid_input"
    );
}
