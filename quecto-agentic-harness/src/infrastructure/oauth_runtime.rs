//! Infrastructure helpers for OAuth refreshable provider runtimes.

use std::sync::Arc;

use crate::infrastructure::auth::credential_store::Credential;
use crate::infrastructure::providers::refreshable::{ProviderFactory, RefreshFn};

/// Build a [`RefreshFn`] for use with refreshable providers.
pub fn make_oauth_refresh_fn() -> RefreshFn {
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
            sync_credentials_to_manager(store.path()).await;
            Ok(token)
        })
    })
}

fn expires_at_with_margin(expires_in: u64) -> i64 {
    crate::infrastructure::time::unix_timestamp_secs() + expires_in as i64 - 300
}

/// Persist a refreshed OAuth token response into the credential store.
pub fn persist_refreshed_token(
    store: &crate::infrastructure::auth::credential_store::CredentialStore,
    provider: &str,
    previous_refresh_token: &str,
    refresh_result: Result<
        crate::infrastructure::auth::oauth::OAuthTokenResponse,
        crate::domain::error::DomainError,
    >,
) -> Option<String> {
    match refresh_result {
        Ok(token_resp) => {
            let expires_at = expires_at_with_margin(token_resp.expires_in);
            let account_id = if provider == "openai" {
                crate::infrastructure::auth::oauth::extract_openai_account_id(
                    &token_resp.access_token,
                )
            } else {
                None
            };
            let effective_refresh = token_resp
                .refresh_token
                .unwrap_or_else(|| previous_refresh_token.to_string());
            let new_cred = Credential {
                provider: provider.to_string(),
                token: token_resp.access_token.clone(),
                method: crate::infrastructure::auth::credential_store::AuthMethod::OAuth,
                expires_at: Some(expires_at),
                refresh_token: Some(effective_refresh),
                account_id,
            };
            match store.store_refreshed(new_cred, previous_refresh_token) {
                Ok(authoritative) => Some(authoritative.token),
                Err(e) => {
                    tracing::warn!("failed to persist refreshed token for {}: {}", provider, e);
                    Some(token_resp.access_token)
                }
            }
        }
        Err(e) => {
            tracing::warn!("failed to refresh OAuth token for {}: {}", provider, e);
            None
        }
    }
}

pub(crate) async fn sync_credentials_to_manager(credentials_path: &std::path::Path) {
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

#[cfg(test)]
#[path = "oauth_runtime_tests.rs"]
mod tests;

/// Build a [`ProviderFactory`] that re-creates a provider with a new API key.
pub fn make_provider_factory(
    provider_name: &str,
    api_base: Option<String>,
    http_client: reqwest::Client,
) -> ProviderFactory {
    use crate::infrastructure::providers;

    let name = provider_name.to_string();
    let base = api_base;
    Arc::new(
        move |new_token: &str| -> Arc<dyn crate::domain::provider::LlmProvider> {
            if name == "openai" {
                let account_id =
                    crate::infrastructure::auth::oauth::extract_openai_account_id(new_token);
                if let Some(acct) = account_id {
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
