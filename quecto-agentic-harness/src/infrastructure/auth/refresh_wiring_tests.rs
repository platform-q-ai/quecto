//! Tests for the OAuth refresh wiring: the refresh closure's error
//! branches (no network reached) and the credential sync's no-op and
//! timeouts.

use super::*;
use std::sync::Arc;

use crate::domain::auth::credential::{AuthMethod, Credential};
use crate::infrastructure::auth::credential_store::CredentialStore;

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

/// #2210 review: a manager that never answers cannot hold the credential
/// push for ever; the request is abandoned at its timeout.
#[tokio::test]
async fn a_credential_push_to_a_silent_manager_times_out() {
    use crate::infrastructure::providers::stream_idle::tests::{bounded, servers};
    let url = servers::never_answering().await;
    let timeout = std::time::Duration::from_millis(200);
    let started = std::time::Instant::now();
    let pushed = bounded(push_credentials(
        &url,
        Some("t".into()),
        "{}".into(),
        timeout,
    ))
    .await;
    let error = pushed.expect_err("a silent manager never answers");
    assert!(error.is_timeout(), "{error}");
    assert!(started.elapsed() >= timeout);
    assert_eq!(CREDENTIAL_SYNC_TIMEOUT, std::time::Duration::from_secs(10));
}

/// The sync itself passes [`CREDENTIAL_SYNC_TIMEOUT`]: against a manager
/// that never answers it returns once that long has passed (paused clock).
#[tokio::test(start_paused = true)]
async fn the_credential_sync_gives_up_after_its_timeout() {
    use crate::infrastructure::providers::stream_idle::tests::{bounded, servers};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("credentials.json");
    std::fs::write(&path, "{}").unwrap();
    let url = servers::never_answering().await;
    let started = tokio::time::Instant::now();
    bounded(sync_credentials_to(&url, None, &path)).await;
    let waited = started.elapsed();
    assert!(waited >= CREDENTIAL_SYNC_TIMEOUT, "{waited:?}");
    assert!(
        waited < CREDENTIAL_SYNC_TIMEOUT + std::time::Duration::from_secs(1),
        "{waited:?}"
    );
}
