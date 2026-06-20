//! Additional coverage-focused unit tests for `shared.rs`.
//!
//! These target previously-uncovered pure-logic branches: prompt assembly,
//! provider/refresh closure construction, native-extension wiring, and the
//! no-OAuth-config resolution path. No real network or subprocess is used.

use super::*;
use std::sync::Arc;

use crate::infrastructure::auth::credential_store::{AuthMethod, Credential, CredentialStore};

// --- append_workflow_prompt ---

#[test]
fn append_workflow_prompt_appends_engine_snippet() {
    let workflow: WorkflowStateHandle = std::sync::Arc::new(std::sync::Mutex::new(
        crate::domain::workflow::WorkflowEngine::new(
            crate::domain::workflow::WorkflowConfig::default(),
            false,
        )
        .unwrap(),
    ));
    let mut system = "base".to_string();
    append_workflow_prompt(&mut system, &workflow);
    assert!(system.starts_with("base\n\n"), "got: {system}");
    assert!(
        system.len() > "base\n\n".len(),
        "snippet should be appended: {system}"
    );
}

// --- resolve_api_key_with_refresh_async: provider without an OAuth config ---

#[tokio::test]
async fn resolve_api_key_with_refresh_async_no_oauth_config_uses_snapshot() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    // "gemini" has no OAuthConfig::for_provider entry -> snapshot fallback path.
    let resolved = resolve_api_key_with_refresh_async("cfg-fallback", &store, "gemini").await;
    assert_eq!(resolved, "cfg-fallback");
}

#[tokio::test]
async fn resolve_api_key_with_refresh_async_no_oauth_config_returns_stored_token() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    store
        .store(Credential {
            provider: "gemini".to_string(),
            token: "stored-token".to_string(),
            method: AuthMethod::Token,
            expires_at: None,
            refresh_token: None,
            account_id: None,
        })
        .unwrap();
    let resolved = resolve_api_key_with_refresh_async("cfg-fallback", &store, "gemini").await;
    assert_eq!(resolved, "stored-token");
}

// --- make_oauth_refresh_fn error branches (no network reached) ---

#[tokio::test]
async fn make_oauth_refresh_fn_errors_when_no_credential() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = Arc::new(CredentialStore::new(tmp.path()));
    let refresh = make_oauth_refresh_fn();
    let err = refresh(store, "anthropic").await.unwrap_err();
    assert!(
        err.to_string().contains("no credential found"),
        "got: {err}"
    );
}

#[tokio::test]
async fn make_oauth_refresh_fn_errors_when_no_refresh_token() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = Arc::new(CredentialStore::new(tmp.path()));
    store
        .store(Credential {
            provider: "anthropic".to_string(),
            token: "tok".to_string(),
            method: AuthMethod::OAuth,
            expires_at: Some(0),
            refresh_token: None,
            account_id: None,
        })
        .unwrap();
    let refresh = make_oauth_refresh_fn();
    let err = refresh(store, "anthropic").await.unwrap_err();
    assert!(err.to_string().contains("no refresh token"), "got: {err}");
}

#[tokio::test]
async fn make_oauth_refresh_fn_errors_when_no_oauth_config() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = Arc::new(CredentialStore::new(tmp.path()));
    store
        .store(Credential {
            provider: "gemini".to_string(),
            token: "tok".to_string(),
            method: AuthMethod::OAuth,
            expires_at: Some(0),
            refresh_token: Some("rt".to_string()),
            account_id: None,
        })
        .unwrap();
    let refresh = make_oauth_refresh_fn();
    let err = refresh(store, "gemini").await.unwrap_err();
    assert!(err.to_string().contains("no OAuth config"), "got: {err}");
}

// --- make_provider_factory ---

#[test]
fn make_provider_factory_builds_anthropic_provider() {
    let factory = make_provider_factory("anthropic", None, reqwest::Client::new());
    let provider = factory("sk-ant-token");
    assert_eq!(provider.name(), "anthropic");
}

#[test]
fn make_provider_factory_openai_plain_token_builds_openai_provider() {
    // A non-JWT token yields no account id, so it falls through to the
    // standard provider constructor rather than the Codex branch.
    let factory = make_provider_factory("openai", None, reqwest::Client::new());
    let provider = factory("plain-non-jwt-token");
    assert_eq!(provider.name(), "openai");
}

#[test]
fn make_provider_factory_invalid_base_falls_back_to_default_base() {
    // A disallowed https host makes the primary build fail; the factory must
    // recover by rebuilding with no custom base instead of panicking.
    let factory = make_provider_factory(
        "anthropic",
        Some("https://evil.not-anthropic.example.com".to_string()),
        reqwest::Client::new(),
    );
    let provider = factory("sk-ant-token");
    assert_eq!(provider.name(), "anthropic");
}

// --- sync_credentials_to_manager early return (no env configured) ---

#[tokio::test]
async fn sync_credentials_to_manager_noop_when_env_unset() {
    // With QUECTO_CREDENTIAL_SYNC_URL unset this returns immediately without
    // touching the filesystem or network.
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("credentials.json");
    if std::env::var("QUECTO_CREDENTIAL_SYNC_URL").is_err() {
        sync_credentials_to_manager(&path).await;
    }
}

// --- native extension build + registration ---

#[test]
fn build_and_register_native_extensions_registers_web_fetch() {
    let mut config = crate::infrastructure::config::Config::default();
    config.tools.web.fetch.enabled = true;
    let client = reqwest::Client::new();

    let ext_registry = build_and_register_native_extensions(&config, &client);
    let tools = ext_registry.all_tools();
    assert!(
        !tools.is_empty(),
        "fetch.enabled should yield at least one tool"
    );

    let mut tool_registry = crate::infrastructure::tools::registry::ToolRegistryImpl::new();
    register_extension_tools(&mut tool_registry, &ext_registry);
    assert!(
        !tool_registry.extension_names().is_empty(),
        "extension tools should be registered"
    );
}

#[test]
fn build_and_register_native_extensions_empty_when_no_web_tools() {
    let config = crate::infrastructure::config::Config::default();
    let client = reqwest::Client::new();
    let ext_registry = build_and_register_native_extensions(&config, &client);
    assert!(
        ext_registry.all_tools().is_empty(),
        "no web tools enabled -> no extensions"
    );

    // register_extension_tools over an empty registry is a no-op loop.
    let mut tool_registry = crate::infrastructure::tools::registry::ToolRegistryImpl::new();
    register_extension_tools(&mut tool_registry, &ext_registry);
}

// --- resolve_api_key_with_refresh (sync wrapper) outside a runtime ---

#[test]
fn resolve_api_key_with_refresh_sync_wrapper_no_oauth_config() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    let rt = tokio::runtime::Runtime::new().unwrap();
    // "gemini" -> no OAuth config -> snapshot fallback -> config key.
    let resolved = resolve_api_key_with_refresh("cfg-key", &store, "gemini", &rt);
    assert_eq!(resolved, "cfg-key");
}
