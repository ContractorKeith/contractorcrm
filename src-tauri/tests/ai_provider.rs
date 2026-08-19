//! Integration tests for the AI provider seam: settings persistence, the
//! credential seam (never a real keychain), the disabled invariant, and the
//! adapter's failure behavior against real sockets.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

use contractorcrm_lib::ai::{
    clear_ai_api_key, configured_provider, get_ai_settings, provider_for_connection_test,
    set_ai_api_key, set_ai_settings, CompletionProvider, CredentialStore, InMemoryCredentialStore,
    OpenAiCompatibleProvider, ProviderRequest, RecordRef, SetAiSettingsRequest,
    AI_SETTINGS_VERSION,
};
use contractorcrm_lib::domain::Actor;
use contractorcrm_lib::storage::Storage;

fn open_storage(temp: &tempfile::TempDir) -> Storage {
    Storage::open_in_app_data(temp.path()).expect("open storage")
}

fn local_settings(enabled: bool, base_url: &str) -> SetAiSettingsRequest {
    SetAiSettingsRequest {
        actor: Actor::User,
        enabled,
        provider_label: "Local model".into(),
        base_url: base_url.into(),
        model: "llama3.1".into(),
    }
}

/// Bind a port and drop it, so connecting to it is refused. The OS could in
/// principle hand the same ephemeral port to another test's stub server in the
/// window after this returns; nothing in the process reuses a released port
/// deliberately, so treat a surprise success here as that rare collision.
fn closed_port_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind probe port");
    let port = listener.local_addr().expect("probe address").port();
    drop(listener);
    format!("http://127.0.0.1:{port}/v1")
}

/// One-shot HTTP server that answers with the given raw body and 200 OK.
///
/// The request is read to its end before anything is written, and the socket is
/// half-closed and drained afterwards. Closing a socket with an unread request
/// still in its buffer sends a TCP reset, which the client sees instead of the
/// response — the source of intermittent `provider_unavailable` failures in
/// these tests under load.
fn serve_once(body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub server");
    let port = listener.local_addr().expect("stub address").port();
    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
            read_whole_request(&mut stream);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
            // Half-close, then drain: the client learns the response ended and
            // no reset is sent for bytes still in flight.
            let _ = stream.shutdown(Shutdown::Write);
            let _ = stream.read_to_end(&mut Vec::new());
        }
    });
    // The socket is already bound, so the client's connection queues even if
    // the accept loop has not been scheduled yet.
    format!("http://127.0.0.1:{port}/v1")
}

/// Read headers plus the body the request declares, so nothing is left unread.
fn read_whole_request(stream: &mut TcpStream) {
    let mut request = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let Ok(read) = stream.read(&mut chunk) else {
            return;
        };
        if read == 0 {
            return;
        }
        request.extend_from_slice(&chunk[..read]);
        let Some(header_end) = find_header_end(&request) else {
            continue;
        };
        let headers = String::from_utf8_lossy(&request[..header_end]).to_lowercase();
        let declared = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        if request.len() - header_end >= declared {
            return;
        }
    }
}

/// Index just past the blank line that ends the request headers.
fn find_header_end(request: &[u8]) -> Option<usize> {
    request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|start| start + 4)
}

#[test]
fn ai_settings_default_to_a_disabled_local_provider_without_touching_the_keychain() {
    let temp = tempfile::tempdir().expect("temp dir");
    let storage = open_storage(&temp);
    let credentials = InMemoryCredentialStore::new();

    let settings = get_ai_settings(&storage, &credentials).expect("read defaults");

    assert_eq!(settings.version, AI_SETTINGS_VERSION);
    assert!(!settings.enabled);
    assert_eq!(settings.provider_label, "Local model");
    assert_eq!(settings.base_url, "http://127.0.0.1:11434/v1");
    assert_eq!(settings.model, "");
    assert!(!settings.has_api_key);
    assert_eq!(
        credentials.read_count(),
        0,
        "an unconfigured assistant must not read credentials"
    );
}

#[test]
fn ai_settings_round_trip_through_app_settings() {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut storage = open_storage(&temp);
    let credentials = InMemoryCredentialStore::new();

    let saved = set_ai_settings(
        &mut storage,
        &credentials,
        local_settings(true, "http://127.0.0.1:11434/v1/"),
    )
    .expect("save settings");
    assert!(saved.enabled);
    // Trailing slashes are normalized away so URL joining stays predictable.
    assert_eq!(saved.base_url, "http://127.0.0.1:11434/v1");

    let reloaded = get_ai_settings(&storage, &credentials).expect("reload settings");
    assert_eq!(reloaded, saved);

    // The settings row holds the versioned JSON and nothing secret.
    let stored: String = storage
        .connection()
        .query_row(
            "SELECT value FROM app_settings WHERE key = 'ai.provider'",
            [],
            |row| row.get(0),
        )
        .expect("settings row exists");
    assert!(stored.contains("\"version\":1"));
    assert!(!stored.contains("apiKey"));
}

#[test]
fn turning_the_assistant_on_requires_an_endpoint_and_a_model() {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut storage = open_storage(&temp);
    let credentials = InMemoryCredentialStore::new();

    let mut request = local_settings(true, "");
    let error = set_ai_settings(&mut storage, &credentials, request.clone())
        .expect_err("blank base URL is rejected");
    assert_eq!(error.kind(), "invalid_input");

    request.base_url = "http://127.0.0.1:11434/v1".into();
    request.model = String::new();
    let error =
        set_ai_settings(&mut storage, &credentials, request).expect_err("blank model is rejected");
    assert_eq!(error.kind(), "invalid_input");
}

#[test]
fn the_api_key_lives_only_in_the_credential_store() {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut storage = open_storage(&temp);
    let credentials = InMemoryCredentialStore::new();
    set_ai_settings(
        &mut storage,
        &credentials,
        local_settings(true, "https://api.example.com/v1"),
    )
    .expect("save settings");

    let settings = set_ai_api_key(
        &mut storage,
        &credentials,
        Actor::User,
        "sk-secret-key".into(),
    )
    .expect("store key");
    assert!(settings.has_api_key);
    assert_eq!(
        credentials.get_api_key().expect("read key"),
        Some("sk-secret-key".to_owned())
    );

    // Not in app_settings, and not in any command_log summary.
    let settings_rows: String = storage
        .connection()
        .query_row(
            "SELECT COALESCE(group_concat(key || '=' || value), '') FROM app_settings",
            [],
            |row| row.get(0),
        )
        .expect("read app_settings");
    assert!(!settings_rows.contains("sk-secret-key"));
    let summaries: String = storage
        .connection()
        .query_row(
            "SELECT COALESCE(group_concat(summary), '') FROM command_log",
            [],
            |row| row.get(0),
        )
        .expect("read command log");
    assert!(!summaries.contains("sk-secret-key"));
    assert!(summaries.contains("saved the AI provider API key"));
    assert!(!serde_json::to_string(&settings)
        .expect("serialize settings")
        .contains("sk-secret-key"));

    let cleared = clear_ai_api_key(&mut storage, &credentials, Actor::User).expect("clear key");
    assert!(!cleared.has_api_key);
    assert_eq!(credentials.get_api_key().expect("read cleared key"), None);
    // Clearing twice is safe.
    clear_ai_api_key(&mut storage, &credentials, Actor::User).expect("clear again");
}

#[test]
fn a_blank_api_key_is_rejected_before_it_reaches_the_credential_store() {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut storage = open_storage(&temp);
    let credentials = InMemoryCredentialStore::new();

    let error = set_ai_api_key(&mut storage, &credentials, Actor::User, "   ".into())
        .expect_err("blank key rejected");
    assert_eq!(error.kind(), "invalid_input");
    assert_eq!(credentials.get_api_key().expect("still empty"), None);
}

#[test]
fn a_disabled_assistant_reaches_no_provider_and_no_credential_store() {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut storage = open_storage(&temp);
    let credentials = InMemoryCredentialStore::with_key("sk-secret-key");
    set_ai_settings(
        &mut storage,
        &credentials,
        local_settings(false, "http://127.0.0.1:11434/v1"),
    )
    .expect("save disabled settings");

    let before = credentials.read_count();
    assert!(configured_provider(&storage, &credentials)
        .expect("no provider while disabled")
        .is_none());
    let error = provider_for_connection_test(&storage, &credentials)
        .err()
        .expect("testing needs the toggle on");
    assert_eq!(error.kind(), "invalid_input");
    assert_eq!(
        credentials.read_count(),
        before,
        "the disabled path must not read the credential store"
    );
}

#[test]
fn an_enabled_assistant_builds_a_provider_with_the_stored_key() {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut storage = open_storage(&temp);
    let credentials = InMemoryCredentialStore::with_key("sk-secret-key");
    set_ai_settings(
        &mut storage,
        &credentials,
        local_settings(true, "https://api.example.com/v1"),
    )
    .expect("save settings");

    let provider = configured_provider(&storage, &credentials)
        .expect("build provider")
        .expect("provider is configured");
    // A provider borrows nothing from storage, so callers can (and must)
    // release the storage lock before any network I/O.
    drop(storage);
    let call = provider.completion_call(&ProviderRequest::new(
        "draft_follow_up",
        "You draft follow-ups.",
        "Draft a follow-up.",
    ));
    assert!(call.headers.contains(&(
        "Authorization".to_owned(),
        "Bearer sk-secret-key".to_owned()
    )));
}

#[test]
fn a_refused_connection_reports_provider_unavailable() {
    let provider =
        OpenAiCompatibleProvider::new("Local model", closed_port_url(), "llama3.1", None);

    let error = provider.check().expect_err("closed port cannot answer");
    assert_eq!(error.kind(), "provider_unavailable");
    assert!(error.to_string().contains("127.0.0.1"));

    let error = provider
        .complete(&ProviderRequest {
            timeout_seconds: Some(2),
            ..ProviderRequest::new("draft_follow_up", "system", "user")
        })
        .expect_err("closed port cannot answer");
    assert_eq!(error.kind(), "provider_unavailable");
}

#[test]
fn a_garbage_response_reports_provider_unavailable_instead_of_panicking() {
    let provider = OpenAiCompatibleProvider::new(
        "Local model",
        serve_once("not json at all"),
        "llama3.1",
        None,
    );

    let error = provider
        .complete(&ProviderRequest {
            timeout_seconds: Some(5),
            ..ProviderRequest::new("draft_follow_up", "system", "user")
        })
        .expect_err("garbage cannot be decoded");
    assert_eq!(error.kind(), "provider_unavailable");
}

#[test]
fn a_json_response_without_a_completion_reports_provider_unavailable() {
    let provider = OpenAiCompatibleProvider::new(
        "Local model",
        serve_once("{\"error\":\"nope\"}"),
        "llama3.1",
        None,
    );

    let error = provider
        .complete(&ProviderRequest {
            timeout_seconds: Some(5),
            ..ProviderRequest::new("draft_follow_up", "system", "user")
        })
        .expect_err("no completion text");
    assert_eq!(error.kind(), "provider_unavailable");
}

#[test]
fn a_successful_completion_returns_the_text_and_the_records_that_were_included() {
    let base_url = serve_once(
        "{\"model\":\"llama3.1\",\"choices\":[{\"message\":{\"content\":\"Call Jane back.\"}}]}",
    );
    let provider = OpenAiCompatibleProvider::new("Local model", base_url, "llama3.1", None);

    let completion = provider
        .complete(&ProviderRequest {
            included_record_refs: vec![RecordRef {
                entity_type: "contact".into(),
                entity_id: "contact-1".into(),
                label: "Jane Doe".into(),
            }],
            timeout_seconds: Some(5),
            ..ProviderRequest::new("draft_follow_up", "system", "user")
        })
        .expect("completion");

    assert_eq!(completion.text, "Call Jane back.");
    assert_eq!(completion.model, "llama3.1");
    assert_eq!(completion.purpose, "draft_follow_up");
    assert_eq!(completion.included_record_refs.len(), 1);
}

#[test]
fn the_connection_test_never_lists_more_than_fifty_models() {
    // 60 models offered; a picker the user has to scroll forever is no help.
    let listed = (0..60)
        .map(|index| format!("{{\"id\":\"model-{index}\"}}"))
        .collect::<Vec<_>>()
        .join(",");
    let body: &'static str = Box::leak(format!("{{\"data\":[{listed}]}}").into_boxed_str());
    let provider = OpenAiCompatibleProvider::new("Local model", serve_once(body), "model-0", None);

    let check = provider.check().expect("connection test");

    assert_eq!(check.available_models.len(), 50);
    assert_eq!(check.available_models[0], "model-0");
    assert!(check.model_available);
}

#[test]
fn the_connection_test_reports_the_endpoint_and_the_models_it_offers() {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut storage = open_storage(&temp);
    let credentials = InMemoryCredentialStore::new();
    let base_url = serve_once("{\"data\":[{\"id\":\"llama3.1\"},{\"id\":\"mistral\"}]}");
    set_ai_settings(&mut storage, &credentials, local_settings(true, &base_url))
        .expect("save settings");

    // Mirrors the Tauri command: build the provider from storage, release
    // storage entirely, and only then touch the network.
    let provider = provider_for_connection_test(&storage, &credentials).expect("build provider");
    drop(storage);
    drop(temp);
    let check = provider.check().expect("connection test");

    assert_eq!(check.provider_label, "Local model");
    assert!(check.local, "127.0.0.1 endpoints are local");
    assert!(check.model_available);
    assert_eq!(check.available_models, ["llama3.1", "mistral"]);
    assert!(check.endpoint_host.starts_with("127.0.0.1:"));
}
