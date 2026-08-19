//! AI provider seam.
//!
//! Three separate concerns live here, deliberately narrow so later slices
//! (drafting, explanations, the agent interface) reuse them without widening
//! the surface:
//!
//! * `CompletionProvider` — the one interface every model provider implements.
//! * `OpenAiCompatibleProvider` — a blocking chat-completions adapter that
//!   covers local servers (Ollama, LM Studio, llama.cpp) and BYOK cloud
//!   endpoints, which speak the same shape.
//! * settings and credentials — non-secret configuration in `app_settings`,
//!   the API key in the OS credential store. Keys never reach SQLite, the
//!   command log, logs, error messages, or any serialized response.
//!
//! No streaming in v1: every call is one request in, one completion out.
//!
//! Mutex rule — network I/O must never run while the storage mutex is held.
//! A hung endpoint would block every other command for the whole timeout, so
//! the seam is split in two: the functions here read settings and credentials
//! and hand back a self-owned `OpenAiCompatibleProvider`; the caller drops the
//! storage guard and only then calls `check`/`complete` on it. Providers
//! borrow nothing from `Storage`, which keeps that order natural.

use std::time::Duration;

use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::application::{immediate, log_command};
use crate::domain::Actor;
use crate::error::ApplicationError;
use crate::storage::Storage;

/// app_settings key holding the versioned, non-secret provider configuration.
const AI_SETTINGS_KEY: &str = "ai.provider";

/// Schema version of the stored settings blob.
pub const AI_SETTINGS_VERSION: u32 = 1;

/// Service name for the OS credential store, namespaced to this app.
pub const CREDENTIAL_SERVICE: &str = "com.contractorcrm.desktop";

/// Account name for the single provider API key entry.
pub const CREDENTIAL_ACCOUNT: &str = "ai-provider-api-key";

/// Default request timeout; local models are slower than cloud ones, and a
/// hung endpoint must never block the UI thread forever.
pub const DEFAULT_TIMEOUT_SECONDS: u64 = 60;

/// Upper bound so a bad setting cannot wedge a command.
const MAX_TIMEOUT_SECONDS: u64 = 300;

/// Most model lists we will ever show in the connection test.
const MAX_LISTED_MODELS: usize = 50;

// ---------------------------------------------------------------------------
// Provider interface
// ---------------------------------------------------------------------------

/// One record fed into a provider call, so the user can always see exactly
/// which contacts/opportunities left the machine.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordRef {
    pub entity_type: String,
    pub entity_id: String,
    pub label: String,
}

/// A single, inspectable model request. `purpose` names the feature asking
/// (for example "explain_attention_flag"); `included_record_refs` is the
/// disclosure list shown before the call is made.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRequest {
    pub purpose: String,
    pub system_text: String,
    pub user_text: String,
    /// Bounded projection text — never raw database rows.
    pub context_text: Option<String>,
    pub included_record_refs: Vec<RecordRef>,
    pub max_output_tokens: Option<u32>,
    /// Per-call override; absent uses `DEFAULT_TIMEOUT_SECONDS`.
    pub timeout_seconds: Option<u64>,
}

impl ProviderRequest {
    /// Minimal constructor for callers with no record context.
    pub fn new(
        purpose: impl Into<String>,
        system_text: impl Into<String>,
        user_text: impl Into<String>,
    ) -> Self {
        Self {
            purpose: purpose.into(),
            system_text: system_text.into(),
            user_text: user_text.into(),
            ..Self::default()
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(
            self.timeout_seconds
                .unwrap_or(DEFAULT_TIMEOUT_SECONDS)
                .clamp(1, MAX_TIMEOUT_SECONDS),
        )
    }
}

/// Exactly what an AI-backed feature would send, without sending it: the
/// bounded projection text plus the records it names. The agent interface
/// returns this from `preview_context` so a client can inspect the data
/// before any provider call is made.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextPreview {
    /// The feature the preview is for, e.g. "summarize_history".
    pub purpose: String,
    pub context_text: String,
    pub included_record_refs: Vec<RecordRef>,
}

/// A completion plus the provenance the UI needs to display it honestly.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCompletion {
    pub purpose: String,
    pub model: String,
    pub text: String,
    pub included_record_refs: Vec<RecordRef>,
}

/// Result of an explicit connection check.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCheck {
    pub provider_label: String,
    pub endpoint_host: String,
    /// True when the endpoint is on this machine — no data leaves the device.
    pub local: bool,
    pub model: String,
    /// Whether the configured model appeared in the endpoint's model list.
    pub model_available: bool,
    pub available_models: Vec<String>,
}

/// The narrow interface every provider implements. Blocking on purpose: the
/// application core is synchronous and one call answers one request.
pub trait CompletionProvider: Send + Sync {
    /// Human-readable provider name for disclosure lines.
    fn label(&self) -> &str;

    /// Base URL this provider sends to, when it has one. Callers use it for
    /// the "sent to <host>" disclosure; providers that send nowhere (the test
    /// doubles) leave it absent.
    fn endpoint(&self) -> Option<&str> {
        None
    }

    /// Send one request and return the completion text.
    fn complete(&self, request: &ProviderRequest) -> Result<ProviderCompletion, ApplicationError>;

    /// Explicit reachability check; never called implicitly.
    fn check(&self) -> Result<ProviderCheck, ApplicationError>;
}

// ---------------------------------------------------------------------------
// OpenAI-compatible adapter
// ---------------------------------------------------------------------------

/// A prepared HTTP call. Built as pure data so tests can assert the URL,
/// headers, and body — in particular that the API key is only ever a header.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpCall {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<serde_json::Value>,
}

/// Chat-completions adapter for any OpenAI-compatible endpoint: Ollama,
/// LM Studio, llama.cpp server, or a BYOK cloud provider.
pub struct OpenAiCompatibleProvider {
    label: String,
    base_url: String,
    model: String,
    api_key: Option<String>,
}

impl OpenAiCompatibleProvider {
    pub fn new(
        label: impl Into<String>,
        base_url: impl Into<String>,
        model: impl Into<String>,
        api_key: Option<String>,
    ) -> Self {
        Self {
            label: label.into(),
            base_url: base_url.into(),
            model: model.into(),
            api_key,
        }
    }

    /// Configured endpoint, so callers can build the "sent to <host>"
    /// disclosure without re-reading settings.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Build the chat-completions call. The key rides in the Authorization
    /// header only; the body carries model text and nothing else.
    pub fn completion_call(&self, request: &ProviderRequest) -> HttpCall {
        let mut messages = Vec::new();
        if !request.system_text.trim().is_empty() {
            messages.push(serde_json::json!({
                "role": "system",
                "content": request.system_text,
            }));
        }
        messages.push(serde_json::json!({
            "role": "user",
            "content": user_message(request),
        }));

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": messages,
            "stream": false,
        });
        if let Some(max_tokens) = request.max_output_tokens {
            body["max_tokens"] = serde_json::json!(max_tokens);
        }

        HttpCall {
            method: "POST",
            url: join_url(&self.base_url, "chat/completions"),
            headers: self.headers(true),
            body: Some(body),
        }
    }

    /// Build the model-listing call used by the connection test.
    pub fn models_call(&self) -> HttpCall {
        HttpCall {
            method: "GET",
            url: join_url(&self.base_url, "models"),
            headers: self.headers(false),
            body: None,
        }
    }

    fn headers(&self, with_body: bool) -> Vec<(String, String)> {
        let mut headers = vec![("Accept".to_owned(), "application/json".to_owned())];
        if with_body {
            headers.push(("Content-Type".to_owned(), "application/json".to_owned()));
        }
        // Only attach credentials when the user actually configured a key —
        // local endpoints need none.
        if let Some(api_key) = self.api_key.as_ref() {
            headers.push(("Authorization".to_owned(), format!("Bearer {api_key}")));
        }
        headers
    }
}

impl CompletionProvider for OpenAiCompatibleProvider {
    fn label(&self) -> &str {
        &self.label
    }

    fn endpoint(&self) -> Option<&str> {
        Some(&self.base_url)
    }

    fn complete(&self, request: &ProviderRequest) -> Result<ProviderCompletion, ApplicationError> {
        let call = self.completion_call(request);
        let response = send(&call, request.timeout(), &self.base_url)?;
        let text = response["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| ApplicationError::ProviderUnavailable {
                reason: format!(
                    "{} returned a response ContractorCRM could not read.",
                    endpoint_host(&self.base_url)
                ),
            })?
            .to_owned();
        let model = response["model"].as_str().unwrap_or(&self.model).to_owned();
        Ok(ProviderCompletion {
            purpose: request.purpose.clone(),
            model,
            text,
            included_record_refs: request.included_record_refs.clone(),
        })
    }

    fn check(&self) -> Result<ProviderCheck, ApplicationError> {
        let call = self.models_call();
        let response = send(
            &call,
            Duration::from_secs(DEFAULT_TIMEOUT_SECONDS.min(30)),
            &self.base_url,
        )?;
        let available_models = response["data"]
            .as_array()
            .ok_or_else(|| ApplicationError::ProviderUnavailable {
                reason: format!(
                    "{} did not return a model list ContractorCRM could read.",
                    endpoint_host(&self.base_url)
                ),
            })?
            .iter()
            .filter_map(|model| model["id"].as_str().map(str::to_owned))
            .take(MAX_LISTED_MODELS)
            .collect::<Vec<_>>();
        Ok(ProviderCheck {
            provider_label: self.label.clone(),
            endpoint_host: endpoint_host(&self.base_url),
            local: is_local_endpoint(&self.base_url),
            model: self.model.clone(),
            model_available: available_models.iter().any(|id| id == &self.model),
            available_models,
        })
    }
}

/// Compose the user message: the ask, then the bounded context projection,
/// then the disclosure list of records that were included.
fn user_message(request: &ProviderRequest) -> String {
    let mut message = request.user_text.clone();
    if let Some(context) = request
        .context_text
        .as_ref()
        .filter(|text| !text.trim().is_empty())
    {
        message.push_str("\n\nContext:\n");
        message.push_str(context);
    }
    if !request.included_record_refs.is_empty() {
        message.push_str("\n\nRecords included:");
        for record in &request.included_record_refs {
            message.push_str(&format!("\n- {} ({})", record.label, record.entity_type));
        }
    }
    message
}

/// Send a prepared call with a hard timeout. Every transport, status, and
/// decode failure becomes `provider_unavailable` — this never panics and
/// never echoes credentials.
fn send(
    call: &HttpCall,
    timeout: Duration,
    base_url: &str,
) -> Result<serde_json::Value, ApplicationError> {
    let host = endpoint_host(base_url);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .build()
        .into();

    // POST and GET builders are distinct types in ureq 3, so each branch
    // applies the headers and sends on its own.
    let sent = match &call.body {
        Some(body) => {
            let mut builder = agent.post(&call.url);
            for (name, value) in &call.headers {
                builder = builder.header(name, value);
            }
            builder.send_json(body)
        }
        None => {
            let mut builder = agent.get(&call.url);
            for (name, value) in &call.headers {
                builder = builder.header(name, value);
            }
            builder.call()
        }
    };
    let mut response = sent.map_err(|error| ApplicationError::ProviderUnavailable {
        reason: describe_transport_error(&host, &error),
    })?;
    response
        .body_mut()
        .read_json::<serde_json::Value>()
        .map_err(|_| ApplicationError::ProviderUnavailable {
            reason: format!("{host} returned a response ContractorCRM could not read."),
        })
}

/// Contractor-facing failure text. Deliberately built from the error kind and
/// the host, never from response bodies or request headers.
fn describe_transport_error(host: &str, error: &ureq::Error) -> String {
    match error {
        ureq::Error::StatusCode(401) | ureq::Error::StatusCode(403) => {
            format!("{host} rejected the API key.")
        }
        ureq::Error::StatusCode(404) => {
            format!("{host} has no chat endpoint at that address — check the base URL.")
        }
        ureq::Error::StatusCode(status) => format!("{host} refused the request (HTTP {status})."),
        ureq::Error::Timeout(_) => format!("{host} did not answer in time."),
        _ => format!("Couldn't reach {host}."),
    }
}

/// Join a base URL and a path with exactly one slash between them.
fn join_url(base_url: &str, path: &str) -> String {
    format!("{}/{}", base_url.trim_end_matches('/'), path)
}

/// Host (with port) of the configured endpoint, for disclosure lines and
/// error text. Falls back to the raw value when it is not a URL.
pub fn endpoint_host(base_url: &str) -> String {
    let without_scheme = base_url
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(base_url);
    let authority = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(without_scheme);
    let authority = authority
        .rsplit_once('@')
        .map(|(_, host)| host)
        .unwrap_or(authority);
    if authority.is_empty() {
        base_url.to_owned()
    } else {
        authority.to_owned()
    }
}

/// True when the endpoint runs on this machine, so no CRM data leaves it.
pub fn is_local_endpoint(base_url: &str) -> bool {
    let host = endpoint_host(base_url);
    let host = host.rsplit_once(':').map(|(name, _)| name).unwrap_or(&host);
    let host = host.trim_matches(['[', ']']);
    matches!(host, "localhost" | "127.0.0.1" | "::1" | "0.0.0.0") || host.ends_with(".localhost")
}

// ---------------------------------------------------------------------------
// Credential storage
// ---------------------------------------------------------------------------

/// The provider API key, stored outside CRM data. Implementations must never
/// return the key in errors or logs.
pub trait CredentialStore: Send + Sync {
    fn get_api_key(&self) -> Result<Option<String>, ApplicationError>;
    fn set_api_key(&self, api_key: &str) -> Result<(), ApplicationError>;
    fn delete_api_key(&self) -> Result<(), ApplicationError>;
}

/// Production store: the macOS Keychain / Windows Credential Manager entry
/// for this app.
pub struct KeyringCredentialStore {
    service: String,
    account: String,
}

impl KeyringCredentialStore {
    pub fn new() -> Self {
        Self {
            service: CREDENTIAL_SERVICE.to_owned(),
            account: CREDENTIAL_ACCOUNT.to_owned(),
        }
    }

    fn entry(&self) -> Result<keyring::Entry, ApplicationError> {
        keyring::Entry::new(&self.service, &self.account).map_err(keychain_error)
    }
}

impl Default for KeyringCredentialStore {
    fn default() -> Self {
        Self::new()
    }
}

/// A keychain we cannot reach means the provider cannot be used, so it maps
/// onto the same `provider_unavailable` kind the UI already handles.
fn keychain_error(error: keyring::Error) -> ApplicationError {
    ApplicationError::ProviderUnavailable {
        reason: format!("The system credential store could not be reached: {error}"),
    }
}

impl CredentialStore for KeyringCredentialStore {
    fn get_api_key(&self) -> Result<Option<String>, ApplicationError> {
        match self.entry()?.get_password() {
            Ok(password) => Ok(Some(password)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(keychain_error(error)),
        }
    }

    fn set_api_key(&self, api_key: &str) -> Result<(), ApplicationError> {
        self.entry()?.set_password(api_key).map_err(keychain_error)
    }

    fn delete_api_key(&self) -> Result<(), ApplicationError> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(keychain_error(error)),
        }
    }
}

/// In-memory store used by tests so `cargo test` never touches a real
/// keychain. Also counts reads, which the AI-disabled invariant test asserts.
#[derive(Debug, Default)]
pub struct InMemoryCredentialStore {
    inner: std::sync::Mutex<Option<String>>,
    reads: std::sync::atomic::AtomicUsize,
}

impl InMemoryCredentialStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_key(api_key: &str) -> Self {
        let store = Self::default();
        store
            .set_api_key(api_key)
            .expect("in-memory store always accepts a key");
        store
    }

    /// How many times the key was read — proves the disabled path stays away
    /// from credential storage.
    pub fn read_count(&self) -> usize {
        self.reads.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl CredentialStore for InMemoryCredentialStore {
    fn get_api_key(&self) -> Result<Option<String>, ApplicationError> {
        self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(self
            .inner
            .lock()
            .expect("credential mutex poisoned")
            .clone())
    }

    fn set_api_key(&self, api_key: &str) -> Result<(), ApplicationError> {
        *self.inner.lock().expect("credential mutex poisoned") = Some(api_key.to_owned());
        Ok(())
    }

    fn delete_api_key(&self) -> Result<(), ApplicationError> {
        *self.inner.lock().expect("credential mutex poisoned") = None;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Settings (non-secret) in app_settings
// ---------------------------------------------------------------------------

/// Exactly what is persisted: no key, no derived state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredAiSettings {
    version: u32,
    enabled: bool,
    provider_label: String,
    base_url: String,
    model: String,
}

impl Default for StoredAiSettings {
    fn default() -> Self {
        Self {
            version: AI_SETTINGS_VERSION,
            enabled: false,
            // A local Ollama server is the default assumption: nothing leaves
            // the machine until the user points it somewhere else.
            provider_label: "Local model".to_owned(),
            base_url: "http://127.0.0.1:11434/v1".to_owned(),
            model: String::new(),
        }
    }
}

/// Wire shape returned to the UI: stored settings plus `hasApiKey`, which is
/// derived from the credential store and never persisted.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSettings {
    pub version: u32,
    pub enabled: bool,
    pub provider_label: String,
    pub base_url: String,
    pub model: String,
    pub has_api_key: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetAiSettingsRequest {
    #[serde(default)]
    pub actor: Actor,
    pub enabled: bool,
    pub provider_label: String,
    pub base_url: String,
    pub model: String,
}

/// Read the AI settings. When nothing has ever been configured this returns
/// the defaults without touching the credential store at all.
pub fn get_ai_settings(
    storage: &Storage,
    credentials: &dyn CredentialStore,
) -> Result<AiSettings, ApplicationError> {
    match read_stored_settings(storage)? {
        Some(stored) => {
            let has_api_key = credentials.get_api_key()?.is_some();
            Ok(to_wire(stored, has_api_key))
        }
        None => Ok(to_wire(StoredAiSettings::default(), false)),
    }
}

/// Persist the non-secret provider configuration. Turning the assistant on
/// requires a usable endpoint and model; turning it off keeps whatever the
/// user already typed.
pub fn set_ai_settings(
    storage: &mut Storage,
    credentials: &dyn CredentialStore,
    request: SetAiSettingsRequest,
) -> Result<AiSettings, ApplicationError> {
    let provider_label = validated_label(request.provider_label)?;
    let base_url = validated_base_url(request.base_url, request.enabled)?;
    let model = validated_model(request.model, request.enabled)?;

    let stored = StoredAiSettings {
        version: AI_SETTINGS_VERSION,
        enabled: request.enabled,
        provider_label,
        base_url,
        model,
    };
    write_stored_settings(
        storage,
        request.actor,
        &stored,
        if stored.enabled {
            "turned the AI assistant on"
        } else {
            "turned the AI assistant off"
        },
    )?;

    // Only an enabled provider justifies reading the credential store.
    let has_api_key = stored.enabled && credentials.get_api_key()?.is_some();
    Ok(to_wire(stored, has_api_key))
}

/// Store the provider API key in the OS credential store. The key is never
/// written to SQLite and never appears in the command log summary.
pub fn set_ai_api_key(
    storage: &mut Storage,
    credentials: &dyn CredentialStore,
    actor: Actor,
    api_key: String,
) -> Result<AiSettings, ApplicationError> {
    let api_key = api_key.trim().to_owned();
    if api_key.is_empty() {
        return Err(ApplicationError::InvalidInput {
            field: "apiKey".into(),
            message: "is required".into(),
        });
    }
    if api_key.chars().count() > 500 {
        return Err(ApplicationError::InvalidInput {
            field: "apiKey".into(),
            message: "must be 500 characters or fewer".into(),
        });
    }
    credentials.set_api_key(&api_key)?;

    let stored = read_stored_settings(storage)?.unwrap_or_default();
    write_stored_settings(storage, actor, &stored, "saved the AI provider API key")?;
    Ok(to_wire(stored, true))
}

/// Remove the stored API key. Safe to call when none exists.
pub fn clear_ai_api_key(
    storage: &mut Storage,
    credentials: &dyn CredentialStore,
    actor: Actor,
) -> Result<AiSettings, ApplicationError> {
    credentials.delete_api_key()?;
    let stored = read_stored_settings(storage)?.unwrap_or_default();
    write_stored_settings(storage, actor, &stored, "removed the AI provider API key")?;
    Ok(to_wire(stored, false))
}

/// Prepare an explicit connection check: read the settings and the key, then
/// hand back a self-owned provider. This deliberately does NOT call the
/// network — see the mutex rule at the top of this module. Refuses to touch
/// the network or the credential store while the assistant is switched off.
pub fn provider_for_connection_test(
    storage: &Storage,
    credentials: &dyn CredentialStore,
) -> Result<OpenAiCompatibleProvider, ApplicationError> {
    let stored = read_stored_settings(storage)?.unwrap_or_default();
    if !stored.enabled {
        return Err(ApplicationError::InvalidInput {
            field: "enabled".into(),
            message: "turn the AI assistant on before testing the connection".into(),
        });
    }
    build_provider(&stored, credentials)
}

/// Build the configured provider, attaching the API key only when one is
/// stored. Returns `None`-free `Result` because callers only reach here after
/// the enabled check.
fn build_provider(
    stored: &StoredAiSettings,
    credentials: &dyn CredentialStore,
) -> Result<OpenAiCompatibleProvider, ApplicationError> {
    let api_key = credentials.get_api_key()?;
    Ok(OpenAiCompatibleProvider::new(
        stored.provider_label.clone(),
        stored.base_url.clone(),
        stored.model.clone(),
        api_key,
    ))
}

/// Load the configured provider for other application code. `Ok(None)` means
/// the assistant is off or unconfigured — the caller must not fall back to
/// anything, and nothing was read from the credential store.
///
/// The returned provider borrows nothing, so the caller must release the
/// storage lock before calling `complete`/`check` on it (see the mutex rule
/// at the top of this module).
pub fn configured_provider(
    storage: &Storage,
    credentials: &dyn CredentialStore,
) -> Result<Option<OpenAiCompatibleProvider>, ApplicationError> {
    let Some(stored) = read_stored_settings(storage)? else {
        return Ok(None);
    };
    if !stored.enabled {
        return Ok(None);
    }
    build_provider(&stored, credentials).map(Some)
}

fn to_wire(stored: StoredAiSettings, has_api_key: bool) -> AiSettings {
    AiSettings {
        version: stored.version,
        enabled: stored.enabled,
        provider_label: stored.provider_label,
        base_url: stored.base_url,
        model: stored.model,
        has_api_key,
    }
}

fn read_stored_settings(storage: &Storage) -> Result<Option<StoredAiSettings>, ApplicationError> {
    let value: Option<String> = storage
        .connection()
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            [AI_SETTINGS_KEY],
            |row| row.get(0),
        )
        .ok();
    let Some(value) = value else {
        return Ok(None);
    };
    let stored = serde_json::from_str::<StoredAiSettings>(&value).map_err(|error| {
        ApplicationError::InvalidStoredData(format!(
            "app_settings {AI_SETTINGS_KEY} holds invalid JSON: {error}"
        ))
    })?;
    if stored.version != AI_SETTINGS_VERSION {
        return Err(ApplicationError::InvalidStoredData(format!(
            "app_settings {AI_SETTINGS_KEY} has unsupported version {}",
            stored.version
        )));
    }
    Ok(Some(stored))
}

fn write_stored_settings(
    storage: &mut Storage,
    actor: Actor,
    stored: &StoredAiSettings,
    summary: &str,
) -> Result<(), ApplicationError> {
    let value = serde_json::to_string(stored).map_err(|error| {
        ApplicationError::InvalidStoredData(format!("AI settings could not be encoded: {error}"))
    })?;
    let transaction = immediate(storage)?;
    transaction.execute(
        "INSERT INTO app_settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![AI_SETTINGS_KEY, value],
    )?;
    log_command(&transaction, actor, "settings", "ai", summary)?;
    transaction.commit()?;
    Ok(())
}

fn validated_label(value: String) -> Result<String, ApplicationError> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(StoredAiSettings::default().provider_label);
    }
    if value.chars().count() > 100 {
        return Err(ApplicationError::InvalidInput {
            field: "providerLabel".into(),
            message: "must be 100 characters or fewer".into(),
        });
    }
    Ok(value.to_owned())
}

fn validated_base_url(value: String, required: bool) -> Result<String, ApplicationError> {
    let value = value.trim().trim_end_matches('/').to_owned();
    if value.is_empty() {
        if required {
            return Err(ApplicationError::InvalidInput {
                field: "baseUrl".into(),
                message: "is required to turn on the AI assistant".into(),
            });
        }
        return Ok(value);
    }
    if !value.starts_with("http://") && !value.starts_with("https://") {
        return Err(ApplicationError::InvalidInput {
            field: "baseUrl".into(),
            message: "must start with http:// or https://".into(),
        });
    }
    if value.chars().count() > 2048 {
        return Err(ApplicationError::InvalidInput {
            field: "baseUrl".into(),
            message: "must be 2048 characters or fewer".into(),
        });
    }
    Ok(value)
}

fn validated_model(value: String, required: bool) -> Result<String, ApplicationError> {
    let value = value.trim().to_owned();
    if value.is_empty() && required {
        return Err(ApplicationError::InvalidInput {
            field: "model".into(),
            message: "is required to turn on the AI assistant".into(),
        });
    }
    if value.chars().count() > 200 {
        return Err(ApplicationError::InvalidInput {
            field: "model".into(),
            message: "must be 200 characters or fewer".into(),
        });
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request_with_context() -> ProviderRequest {
        ProviderRequest {
            purpose: "explain_attention_flag".into(),
            system_text: "You summarize CRM facts.".into(),
            user_text: "Why does this lead need attention?".into(),
            context_text: Some("Last call: 2026-07-01".into()),
            included_record_refs: vec![RecordRef {
                entity_type: "contact".into(),
                entity_id: "contact-1".into(),
                label: "Jane Doe".into(),
            }],
            max_output_tokens: Some(256),
            timeout_seconds: None,
        }
    }

    #[test]
    fn completion_call_targets_the_chat_endpoint_with_the_configured_model() {
        let provider = OpenAiCompatibleProvider::new(
            "Local model",
            "http://127.0.0.1:11434/v1/",
            "llama3.1",
            None,
        );
        let call = provider.completion_call(&request_with_context());

        assert_eq!(call.method, "POST");
        assert_eq!(call.url, "http://127.0.0.1:11434/v1/chat/completions");
        let body = call.body.expect("chat calls carry a body");
        assert_eq!(body["model"], "llama3.1");
        assert_eq!(body["stream"], false);
        assert_eq!(body["max_tokens"], 256);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["role"], "user");
        let user_content = body["messages"][1]["content"]
            .as_str()
            .expect("user content is text");
        assert!(user_content.contains("Last call: 2026-07-01"));
        assert!(user_content.contains("Jane Doe (contact)"));
    }

    #[test]
    fn no_authorization_header_without_a_configured_key() {
        let provider = OpenAiCompatibleProvider::new(
            "Local model",
            "http://127.0.0.1:11434/v1",
            "llama3.1",
            None,
        );
        let call = provider.completion_call(&request_with_context());
        assert!(!call.headers.iter().any(|(name, _)| name == "Authorization"));
    }

    #[test]
    fn a_configured_key_rides_in_the_header_and_never_in_the_body() {
        let provider = OpenAiCompatibleProvider::new(
            "Cloud model",
            "https://api.example.com/v1",
            "gpt-test",
            Some("sk-secret-key".into()),
        );
        let call = provider.completion_call(&request_with_context());

        assert!(call.headers.contains(&(
            "Authorization".to_owned(),
            "Bearer sk-secret-key".to_owned()
        )));
        let body = serde_json::to_string(&call.body).expect("serialize body");
        assert!(
            !body.contains("sk-secret-key"),
            "body must not carry the key"
        );
        assert!(
            !call.url.contains("sk-secret-key"),
            "URL must not carry the key"
        );
    }

    #[test]
    fn models_call_lists_models_at_the_configured_base_url() {
        let provider = OpenAiCompatibleProvider::new(
            "Cloud model",
            "https://api.example.com/v1",
            "gpt-test",
            Some("sk-secret-key".into()),
        );
        let call = provider.models_call();
        assert_eq!(call.method, "GET");
        assert_eq!(call.url, "https://api.example.com/v1/models");
        assert!(call.body.is_none());
    }

    #[test]
    fn endpoint_host_and_locality_drive_the_disclosure_line() {
        assert_eq!(
            endpoint_host("http://127.0.0.1:11434/v1"),
            "127.0.0.1:11434"
        );
        assert_eq!(endpoint_host("https://api.openai.com/v1"), "api.openai.com");
        assert!(is_local_endpoint("http://localhost:1234/v1"));
        assert!(is_local_endpoint("http://127.0.0.1:11434/v1"));
        assert!(!is_local_endpoint("https://api.openai.com/v1"));
    }

    #[test]
    fn in_memory_credential_store_round_trips_and_counts_reads() {
        let store = InMemoryCredentialStore::new();
        assert_eq!(store.get_api_key().expect("read empty"), None);
        store.set_api_key("sk-test").expect("store key");
        assert_eq!(
            store.get_api_key().expect("read key"),
            Some("sk-test".to_owned())
        );
        store.delete_api_key().expect("delete key");
        assert_eq!(store.get_api_key().expect("read after delete"), None);
        store.delete_api_key().expect("delete is idempotent");
        assert_eq!(store.read_count(), 3);
    }

    #[test]
    fn enabling_requires_an_endpoint_and_a_model() {
        let error = validated_base_url(String::new(), true).expect_err("base url required");
        assert_eq!(error.kind(), "invalid_input");
        let error = validated_base_url("ftp://models".into(), true).expect_err("scheme checked");
        assert_eq!(error.kind(), "invalid_input");
        let error = validated_model(" ".into(), true).expect_err("model required");
        assert_eq!(error.kind(), "invalid_input");
        assert_eq!(validated_model(String::new(), false).expect("optional"), "");
    }
}
