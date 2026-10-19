//! OAuth refresh wiring (#1849 PR 1; folded into `infrastructure::auth` by
//! #1839): the [`RefreshFn`] that rotates an expired OAuth credential
//! through the kernel's OAuth endpoints, persists it, and pushes the
//! refreshed store to the runtime manager. Handed to the runtime
//! composition; the provider it serves is rebuilt by
//! `infrastructure::providers::routing::provider_factory`. Dependencies run one
//! way: the providers' refresh decorator names this module, never the
//! reverse.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::domain::shared::error::DomainError;
use crate::infrastructure::auth::credential_store::CredentialStore;
use crate::infrastructure::auth::token_refresh::persist_refreshed_token;

/// Async function that refreshes an OAuth token.
///
/// Takes the credential store and provider name, returns the new access token.
/// Responsible for persisting the refreshed credential in the store.
pub type RefreshFn = Arc<
    dyn Fn(
            Arc<CredentialStore>,
            &str,
        ) -> Pin<Box<dyn Future<Output = Result<String, DomainError>> + Send>>
        + Send
        + Sync,
>;

/// Build a [`RefreshFn`] for the providers' refresh decorator
/// (`RefreshableProvider`).
///
/// The returned function reads the stored refresh token, calls the appropriate
/// OAuth refresh endpoint, persists the new credential, and returns the new
/// access token.
pub fn make_oauth_refresh_fn() -> RefreshFn {
    Arc::new(|store, provider_name| {
        let provider_name = provider_name.to_string();
        let store = store.clone();
        Box::pin(async move {
            let creds = store.load_snapshot().unwrap_or_default();
            let cred = creds.get(&provider_name).ok_or_else(|| {
                crate::domain::shared::error::DomainError::Provider(format!(
                    "no credential found for {}",
                    provider_name
                ))
            })?;
            let refresh_token = cred.refresh_token.as_ref().ok_or_else(|| {
                crate::domain::shared::error::DomainError::Provider(format!(
                    "no refresh token for {}",
                    provider_name
                ))
            })?;
            let oauth_config =
                crate::infrastructure::auth::oauth::OAuthConfig::for_provider(&provider_name)
                    .ok_or_else(|| {
                        crate::domain::shared::error::DomainError::Provider(format!(
                            "no OAuth config for {}",
                            provider_name
                        ))
                    })?;

            let refresh_result = match provider_name.as_str() {
                "openai" => {
                    crate::infrastructure::auth::oauth::refresh_openai_token(
                        &oauth_config,
                        refresh_token,
                    )
                    .await
                }
                "xai" => {
                    crate::infrastructure::auth::oauth::refresh_xai_token(
                        &oauth_config,
                        refresh_token,
                    )
                    .await
                }
                _ => {
                    crate::infrastructure::auth::oauth::refresh_anthropic_token(
                        &oauth_config,
                        refresh_token,
                    )
                    .await
                }
            };

            let token =
                persist_refreshed_token(&store, &provider_name, refresh_token, refresh_result)
                    .ok_or_else(|| {
                        crate::domain::shared::error::DomainError::Provider(format!(
                            "failed to refresh token for {}",
                            provider_name
                        ))
                    })?;

            // Best-effort: push the refreshed credentials back to the runtime
            // manager so the shared Secret (and therefore newly spawned pods)
            // start from a fresh, non-expired token. Failure here must not fail
            // the refresh — the in-process token is already valid.
            sync_credentials_to_manager(store.path()).await;

            Ok(token)
        })
    })
}

/// Push the local `credentials.json` to the runtime manager's credential sync
/// endpoint, if configured via `QUECTO_CREDENTIAL_SYNC_URL`.
///
/// Best-effort and non-fatal: any failure is logged and swallowed. When the env
/// var is unset (e.g. local CLI use, no cluster manager), this is a no-op.
async fn sync_credentials_to_manager(credentials_path: &std::path::Path) {
    let Ok(url) = std::env::var("QUECTO_CREDENTIAL_SYNC_URL") else {
        return;
    };
    if url.trim().is_empty() {
        return;
    }
    let token = std::env::var("QUECTO_CREDENTIAL_SYNC_TOKEN").ok();
    sync_credentials_to(&url, token, credentials_path).await;
}

/// Push the local `credentials.json` to the manager at `url`, abandoning the
/// push after [`CREDENTIAL_SYNC_TIMEOUT`]; best-effort, as above.
async fn sync_credentials_to(url: &str, token: Option<String>, credentials_path: &std::path::Path) {
    let credentials_json = match tokio::fs::read_to_string(credentials_path).await {
        Ok(contents) => contents,
        Err(e) => {
            tracing::warn!(error = %e, "credential sync: failed to read credentials file");
            return;
        }
    };

    let pushed = push_credentials(url, token, credentials_json, CREDENTIAL_SYNC_TIMEOUT).await;
    match pushed {
        Ok(resp) if resp.status().is_success() => {
            tracing::info!("credential sync: pushed refreshed credentials to runtime manager");
        }
        Ok(resp) => {
            tracing::warn!(status = %resp.status(), "credential sync: manager rejected update");
        }
        Err(e) => {
            tracing::warn!(error = %e, "credential sync: request to manager failed");
        }
    }
}

/// How long pushing refreshed credentials to the runtime manager may take in
/// all (#2210 review): it is a small local request, so ten seconds is ample,
/// and a manager that never answers must not hold the refresh path for ever.
const CREDENTIAL_SYNC_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// PUT `credentials_json` to the manager at `url`, bearer-authenticated when
/// a non-blank `token` is given, abandoning the request after `timeout`.
async fn push_credentials(
    url: &str,
    token: Option<String>,
    credentials_json: String,
    timeout: std::time::Duration,
) -> Result<reqwest::Response, reqwest::Error> {
    let mut request = reqwest::Client::new()
        .put(url)
        .timeout(timeout)
        .json(&serde_json::json!({ "credentials_json": credentials_json }));
    if let Some(token) = token.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        request = request.bearer_auth(token);
    }
    request.send().await
}

#[cfg(test)]
#[path = "refresh_wiring_tests.rs"]
mod tests;
