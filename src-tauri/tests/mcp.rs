//! Integration tests for the MCP stdio adapter: the handshake, the mode
//! split, error mapping, bounded reads, the propose → apply → undo round trip,
//! and the audit trail. No network and no real keychain are involved — the
//! provider is a canned one and credentials live in memory.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use contractorcrm_lib::ai::{
    set_ai_settings, CompletionProvider, CredentialStore, InMemoryCredentialStore, ProviderCheck,
    ProviderCompletion, ProviderRequest, SetAiSettingsRequest,
};
use contractorcrm_lib::application::{
    self, ActivityPatch, ContactPatch, CreateContactRequest, LogActivityRequest,
};
use contractorcrm_lib::attachments::{
    self, AddAttachmentRequest, AttachmentParentType, AttachmentStore,
};
use contractorcrm_lib::domain::{Actor, Contact};
use contractorcrm_lib::error::ApplicationError;
use contractorcrm_lib::mcp::{Mode, Server, MAX_TIMELINE_BODY_CHARS};
use contractorcrm_lib::storage::Storage;
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

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
