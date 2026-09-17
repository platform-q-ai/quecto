//! Tests for the provider refresh wiring: the refresh closure's error
//! branches (no network reached), the credential-sync no-op, and the
//! provider factory's rebuild and fallback paths.

use super::*;
use std::sync::Arc;

use crate::infrastructure::auth::credential_store::{AuthMethod, Credential, CredentialStore};

fn jwt_with_account_id(account_id: &str) -> String {
    use base64::Engine;
    let payload = serde_json::json!({
        "https://api.openai.com/auth": { "chatgpt_account_id": account_id }
    });
    let enc = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(&payload).unwrap());
    format!("header.{}.signature", enc)
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

// --- make_provider_factory: OpenAI JWT token takes the Codex branch ---

#[test]
fn make_provider_factory_openai_jwt_builds_codex_provider() {
    let factory = make_provider_factory("openai", None, reqwest::Client::new());
    let provider = factory(&jwt_with_account_id("acct-123"));
    // A JWT carrying an account id routes to the Codex provider.
    assert_eq!(provider.name(), "codex");
}

#[test]
fn make_provider_factory_invalid_provider_falls_back_to_openai() {
    let factory = make_provider_factory("not-a-provider", None, reqwest::Client::new());
    let provider = factory("new-token");
    assert_eq!(provider.name(), "openai");
}

#[test]
fn make_provider_factory_retries_codex_with_default_base_for_bad_custom_base() {
    let factory = make_provider_factory(
        "openai",
        Some("http://[::1".to_string()),
        reqwest::Client::new(),
    );
    let provider = factory(&jwt_with_account_id("acct-fallback"));
    assert_eq!(provider.name(), "codex");
}
