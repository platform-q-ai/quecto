//! Additional coverage-focused unit tests for `shared.rs`.
//!
//! These target previously-uncovered pure-logic branches: prompt assembly,
//! provider/refresh closure construction, native-extension wiring, and the
//! no-OAuth-config resolution path. No real network or subprocess is used.

use super::*;
use std::sync::Arc;

use crate::infrastructure::auth::credential_store::{AuthMethod, Credential, CredentialStore};

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

#[test]
fn xai_oauth_config_is_prerequisite_for_refresh_dispatch() {
    // PR #1087: this guards only the *precondition* of the xAI refresh
    // dispatch in make_oauth_refresh_fn — for_provider("xai") must exist and
    // point at the xAI token endpoint, otherwise the closure would fall into
    // the "no OAuth config" branch before reaching the `"xai" =>
    // refresh_xai_token(...)` arm. It does NOT itself execute that arm; the
    // arm's wire behaviour is covered by refresh_xai_token tests in
    // oauth_xai_tests.rs against a mock server. Kept network-free for CI.
    let config = crate::infrastructure::auth::oauth::OAuthConfig::for_provider("xai").unwrap();
    assert_eq!(config.token_url, "https://auth.x.ai/oauth2/token");
    assert!(!config.client_id.is_empty());
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
    // Call unconditionally: with no sync URL configured this is a no-op, and the
    // function never writes to `path` regardless, so the file must not appear.
    sync_credentials_to_manager(&path).await;
    assert!(
        !path.exists(),
        "no-op credential sync must not create the credentials file"
    );
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

// --- resolve_api_key_with_refresh_async_with_oauth_config: remaining branches ---

/// Build a fake JWT whose payload carries an OpenAI chatgpt_account_id, so
/// `extract_openai_account_id` returns `Some`.
fn jwt_with_account_id(account_id: &str) -> String {
    use base64::Engine;
    let payload = serde_json::json!({
        "https://api.openai.com/auth": { "chatgpt_account_id": account_id }
    });
    let enc = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(&payload).unwrap());
    format!("header.{}.signature", enc)
}

#[tokio::test]
async fn with_oauth_config_openai_dispatch_refreshes_and_persists_account_id() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let new_access = jwt_with_account_id("acct-from-refresh");
    let response = serde_json::json!({
        "access_token": new_access,
        "refresh_token": "oai-new-rt",
        "expires_in": 3600
    });
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response))
        .mount(&server)
        .await;

    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    store
        .store(Credential {
            provider: "openai".to_string(),
            token: "oai-expired".to_string(),
            method: AuthMethod::OAuth,
            expires_at: Some(0),
            refresh_token: Some("oai-old-rt".to_string()),
            account_id: None,
        })
        .unwrap();

    let resolved = resolve_api_key_with_refresh_async_with_oauth_config(
        "cfg",
        &store,
        "openai",
        &crate::infrastructure::auth::oauth::OAuthConfig::with_base_url(&server.uri()),
    )
    .await;
    assert_eq!(resolved, new_access);

    // persist_refreshed_token's openai branch should have extracted the account id.
    let creds = store.load_snapshot().unwrap();
    let cred = creds.get("openai").unwrap();
    assert_eq!(cred.account_id.as_deref(), Some("acct-from-refresh"));
    assert_eq!(cred.refresh_token.as_deref(), Some("oai-new-rt"));
}

#[tokio::test]
async fn with_oauth_config_expired_non_oauth_method_falls_back_to_config() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    // Expired but method=Token => refresh branch skipped => config key.
    store
        .store(Credential {
            provider: "anthropic".to_string(),
            token: "stale".to_string(),
            method: AuthMethod::Token,
            expires_at: Some(0),
            refresh_token: Some("rt".to_string()),
            account_id: None,
        })
        .unwrap();

    let resolved = resolve_api_key_with_refresh_async_with_oauth_config(
        "cfg-key-token",
        &store,
        "anthropic",
        &crate::infrastructure::auth::oauth::OAuthConfig::with_base_url("http://127.0.0.1:1"),
    )
    .await;
    assert_eq!(resolved, "cfg-key-token");
}

#[tokio::test]
async fn with_oauth_config_expired_oauth_without_refresh_token_falls_back() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    // Expired OAuth but no refresh token => cannot refresh => config key.
    store
        .store(Credential {
            provider: "anthropic".to_string(),
            token: "expired".to_string(),
            method: AuthMethod::OAuth,
            expires_at: Some(0),
            refresh_token: None,
            account_id: None,
        })
        .unwrap();

    let resolved = resolve_api_key_with_refresh_async_with_oauth_config(
        "cfg-no-rt",
        &store,
        "anthropic",
        &crate::infrastructure::auth::oauth::OAuthConfig::with_base_url("http://127.0.0.1:1"),
    )
    .await;
    assert_eq!(resolved, "cfg-no-rt");
}

#[tokio::test]
async fn with_oauth_config_valid_token_short_circuits() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    store
        .store(Credential {
            provider: "anthropic".to_string(),
            token: "still-valid".to_string(),
            method: AuthMethod::OAuth,
            expires_at: Some(i64::MAX),
            refresh_token: Some("rt".to_string()),
            account_id: None,
        })
        .unwrap();

    let resolved = resolve_api_key_with_refresh_async_with_oauth_config(
        "cfg",
        &store,
        "anthropic",
        &crate::infrastructure::auth::oauth::OAuthConfig::with_base_url("http://127.0.0.1:1"),
    )
    .await;
    assert_eq!(resolved, "still-valid");
}

// --- make_provider_factory: OpenAI JWT token takes the Codex branch ---

#[test]
fn make_provider_factory_openai_jwt_builds_codex_provider() {
    let factory = make_provider_factory("openai", None, reqwest::Client::new());
    let provider = factory(&jwt_with_account_id("acct-123"));
    // A JWT carrying an account id routes to the Codex provider.
    assert_eq!(provider.name(), "codex");
}
