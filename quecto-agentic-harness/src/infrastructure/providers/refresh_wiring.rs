//! Provider refresh wiring (#1849 PR 1, moved out of `interface::shared`):
//! the [`RefreshFn`] that rotates an expired OAuth credential through the
//! kernel's OAuth endpoints and the [`ProviderFactory`] that rebuilds a
//! concrete provider around the fresh token. Both are handed to the runtime
//! composition; neither is constructed by the interface.
//!
//! [`RefreshFn`]: crate::infrastructure::providers::refreshable::RefreshFn
//! [`ProviderFactory`]: crate::infrastructure::providers::refreshable::ProviderFactory

use crate::infrastructure::auth::token_refresh::persist_refreshed_token;

/// Build a [`RefreshFn`] for use with [`RefreshableProvider`].
///
/// [`RefreshableProvider`]: crate::infrastructure::providers::refreshable::RefreshableProvider
///
/// The returned function reads the stored refresh token, calls the appropriate
/// OAuth refresh endpoint, persists the new credential, and returns the new
/// access token.
pub fn make_oauth_refresh_fn() -> crate::infrastructure::providers::refreshable::RefreshFn {
    use std::sync::Arc;
    Arc::new(|store, provider_name| {
        let provider_name = provider_name.to_string();
        let store = store.clone();
        Box::pin(async move {
            let creds = store.load_snapshot().unwrap_or_default();
            let cred = creds.get(&provider_name).ok_or_else(|| {
                crate::domain::error::DomainError::Provider(format!(
                    "no credential found for {}",
                    provider_name
                ))
            })?;
            let refresh_token = cred.refresh_token.as_ref().ok_or_else(|| {
                crate::domain::error::DomainError::Provider(format!(
                    "no refresh token for {}",
                    provider_name
                ))
            })?;
            let oauth_config =
                crate::infrastructure::auth::oauth::OAuthConfig::for_provider(&provider_name)
                    .ok_or_else(|| {
                        crate::domain::error::DomainError::Provider(format!(
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
                        crate::domain::error::DomainError::Provider(format!(
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

    let credentials_json = match tokio::fs::read_to_string(credentials_path).await {
        Ok(contents) => contents,
        Err(e) => {
            tracing::warn!(error = %e, "credential sync: failed to read credentials file");
            return;
        }
    };

    let mut request = reqwest::Client::new()
        .put(&url)
        .json(&serde_json::json!({ "credentials_json": credentials_json }));

    if let Ok(token) = std::env::var("QUECTO_CREDENTIAL_SYNC_TOKEN") {
        let token = token.trim();
        if !token.is_empty() {
            request = request.bearer_auth(token);
        }
    }

    match request.send().await {
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

/// Build a [`ProviderFactory`] that re-creates a provider with a new API key.
///
/// The factory knows the provider name and API base URL, and creates the
/// correct provider type (Codex for OpenAI OAuth, standard otherwise).
pub fn make_provider_factory(
    provider_name: &str,
    api_base: Option<String>,
    http_client: reqwest::Client,
) -> crate::infrastructure::providers::refreshable::ProviderFactory {
    use crate::infrastructure::providers;
    use std::sync::Arc;

    let name = provider_name.to_string();
    let base = api_base;
    Arc::new(
        move |new_token: &str| -> Arc<dyn crate::application::providers::ports::LlmProvider> {
            if name == "openai" {
                let account_id =
                    crate::infrastructure::auth::oauth::extract_openai_account_id(new_token);
                if let Some(acct) = account_id {
                    // `base` was already validated when the original provider
                    // was constructed; an invalid base cannot appear here, but
                    // degrade to the hardwired ChatGPT backend rather than
                    // panic inside the refresh path.
                    match providers::create_codex_provider_with_client(
                        new_token.to_string(),
                        acct.clone(),
                        base.clone(),
                        http_client.clone(),
                    ) {
                        Ok(p) => return p,
                        Err(e) => {
                            tracing::error!(
                                error = %e,
                                "invalid openai api_base at token refresh; using default backend"
                            );
                            return providers::create_codex_provider_with_client(
                                new_token.to_string(),
                                acct,
                                None,
                                http_client.clone(),
                            )
                            .expect("default Codex backend is always valid");
                        }
                    }
                }
            }
            match providers::create_provider_with_client(
                &name,
                new_token.to_string(),
                base.clone(),
                http_client.clone(),
            ) {
                Ok(p) => p,
                Err(e) => {
                    tracing::error!(
                        provider = name.as_str(),
                        error = %e,
                        "failed to rebuild provider after token refresh"
                    );
                    // Return a provider that will fail — better than panicking
                    providers::create_provider_with_client(
                        &name,
                        new_token.to_string(),
                        None,
                        http_client.clone(),
                    )
                    .unwrap_or_else(|_| {
                        Arc::new(
                            crate::infrastructure::providers::openai::OpenAiProvider::new(
                                new_token.to_string(),
                                None,
                            ),
                        )
                    })
                }
            }
        },
    )
}

#[cfg(test)]
#[path = "refresh_wiring_tests.rs"]
mod tests;
