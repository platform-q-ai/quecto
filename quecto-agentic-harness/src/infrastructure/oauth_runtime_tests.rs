use super::*;
use std::sync::Arc;

use crate::infrastructure::auth::credential_store::{AuthMethod, Credential, CredentialStore};

#[test]
fn provider_factory_falls_back_to_default_provider_base_when_configured_base_is_invalid() {
    let client = reqwest::Client::new();

    let openai = make_provider_factory(
        "openai",
        Some("http://example.invalid/v1".to_string()),
        client.clone(),
    );
    let openai_provider = openai("not-a-jwt-token");
    assert_eq!(openai_provider.name(), "openai");

    let anthropic = make_provider_factory(
        "anthropic",
        Some("http://example.invalid".to_string()),
        client,
    );
    let anthropic_provider = anthropic("fresh-token");
    assert_eq!(anthropic_provider.name(), "anthropic");
}

#[test]
fn persist_refreshed_token_updates_store_and_preserves_missing_refresh_token() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());

    let persisted = persist_refreshed_token(
        &store,
        "anthropic",
        "old-refresh",
        Ok(crate::infrastructure::auth::oauth::OAuthTokenResponse {
            access_token: "fresh-access".to_string(),
            refresh_token: None,
            expires_in: 3600,
        }),
    )
    .expect("successful refresh should return an access token");

    assert_eq!(persisted, "fresh-access");
    let saved = store
        .get("anthropic")
        .expect("store readable")
        .expect("credential persisted");
    assert_eq!(saved.token, "fresh-access");
    assert_eq!(saved.refresh_token.as_deref(), Some("old-refresh"));
    assert_eq!(saved.method, AuthMethod::OAuth);
    assert!(saved.expires_at.unwrap() > crate::infrastructure::time::unix_timestamp_secs());
}

#[test]
fn persist_refreshed_token_reports_failed_refresh_as_unusable() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());

    let result = persist_refreshed_token(
        &store,
        "openai",
        "old-refresh",
        Err(crate::domain::error::DomainError::Provider(
            "upstream refresh failed".to_string(),
        )),
    );

    assert!(result.is_none());
    assert!(store.get("openai").unwrap().is_none());
}

#[tokio::test]
async fn oauth_refresh_fn_reports_missing_credentials_before_network_refresh() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = Arc::new(CredentialStore::new(tmp.path()));
    let refresh = make_oauth_refresh_fn();

    let missing = refresh(store.clone(), "openai").await.unwrap_err();
    assert!(
        missing
            .to_string()
            .contains("no credential found for openai")
    );

    store
        .store(Credential {
            provider: "openai".to_string(),
            token: "sk-old".to_string(),
            method: AuthMethod::Token,
            expires_at: None,
            refresh_token: None,
            account_id: None,
        })
        .unwrap();
    let missing_refresh_token = refresh(store.clone(), "openai").await.unwrap_err();
    assert!(
        missing_refresh_token
            .to_string()
            .contains("no refresh token for openai")
    );

    store
        .store(Credential {
            provider: "unknown-oauth".to_string(),
            token: "access".to_string(),
            method: AuthMethod::OAuth,
            expires_at: Some(1),
            refresh_token: Some("refresh".to_string()),
            account_id: None,
        })
        .unwrap();
    let missing_oauth_config = refresh(store, "unknown-oauth").await.unwrap_err();
    assert!(
        missing_oauth_config
            .to_string()
            .contains("no OAuth config for unknown-oauth")
    );
}

#[tokio::test]
async fn sync_credentials_to_manager_returns_when_url_is_unset_or_blank() {
    let prior_url = std::env::var("QUECTO_CREDENTIAL_SYNC_URL").ok();
    // SAFETY: this test temporarily owns this process-wide variable and restores it below.
    unsafe { std::env::remove_var("QUECTO_CREDENTIAL_SYNC_URL") };
    sync_credentials_to_manager(std::path::Path::new("/path/that/need/not/exist")).await;

    // SAFETY: this test temporarily owns this process-wide variable and restores it below.
    unsafe { std::env::set_var("QUECTO_CREDENTIAL_SYNC_URL", "   ") };
    sync_credentials_to_manager(std::path::Path::new("/path/that/need/not/exist")).await;

    match prior_url {
        // SAFETY: restore the process-wide variable saved at test entry.
        Some(value) => unsafe { std::env::set_var("QUECTO_CREDENTIAL_SYNC_URL", value) },
        // SAFETY: restore the process-wide variable to its saved absent state.
        None => unsafe { std::env::remove_var("QUECTO_CREDENTIAL_SYNC_URL") },
    }
}
