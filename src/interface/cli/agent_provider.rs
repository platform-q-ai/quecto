//! Provider construction for the agent CLI.

use std::sync::Arc;

use crate::domain::provider::LlmProvider;
use crate::infrastructure::auth::credential_store::CredentialStore;
use crate::infrastructure::config::Config;
use crate::infrastructure::providers;
use crate::infrastructure::providers::refreshable::{RefreshableConfig, RefreshableProvider};
use crate::infrastructure::providers::router::ProviderRouter;

/// Build a ProviderRouter from config + credential store, suitable for the agent CLI.
///
/// OAuth-backed providers are wrapped in [`RefreshableProvider`] so that
/// expired tokens are automatically refreshed mid-session on 401 (issue #255).
pub fn build_agent_provider(
    config: &Config,
    base_dir: &std::path::Path,
    http_client: &reqwest::Client,
) -> Result<Arc<dyn LlmProvider>, String> {
    let store = CredentialStore::new(base_dir);

    // Build a temporary runtime for token refresh if needed
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("failed to create runtime for token refresh: {}", e))?;

    let mut provider_list: Vec<Arc<dyn crate::domain::provider::LlmProvider>> = Vec::new();
    let store_arc = Arc::new(CredentialStore::new(base_dir));
    let refresh_fn = crate::interface::shared::make_oauth_refresh_fn();

    // Try OpenAI (with auto-refresh for expired OAuth tokens)
    let openai_key = crate::interface::shared::resolve_api_key_with_refresh(
        &config.providers.openai.api_key,
        &store,
        "openai",
        &rt,
    );
    if !openai_key.is_empty() {
        let is_oauth = store.get("openai").ok().flatten().is_some_and(|c| {
            c.method == crate::infrastructure::auth::credential_store::AuthMethod::OAuth
        });
        let openai_base = if config.providers.openai.api_base.is_empty() {
            None
        } else {
            Some(config.providers.openai.api_base.clone())
        };
        let inner = build_single_provider("openai", &openai_key, &openai_base, http_client)?;
        if is_oauth {
            let factory = crate::interface::shared::make_provider_factory(
                "openai",
                openai_base,
                http_client.clone(),
            );
            provider_list.push(Arc::new(RefreshableProvider::new(RefreshableConfig {
                inner,
                store: store_arc.clone(),
                provider_name: "openai".to_string(),
                refresh_fn: refresh_fn.clone(),
                factory,
            })));
        } else {
            provider_list.push(inner);
        }
    }

    // Try Anthropic (with auto-refresh for expired OAuth tokens)
    let anthropic_key = crate::interface::shared::resolve_api_key_with_refresh(
        &config.providers.anthropic.api_key,
        &store,
        "anthropic",
        &rt,
    );
    if !anthropic_key.is_empty() {
        let is_oauth = store.get("anthropic").ok().flatten().is_some_and(|c| {
            c.method == crate::infrastructure::auth::credential_store::AuthMethod::OAuth
        });
        let anthropic_base = if config.providers.anthropic.api_base.is_empty() {
            None
        } else {
            Some(config.providers.anthropic.api_base.clone())
        };
        let inner =
            build_single_provider("anthropic", &anthropic_key, &anthropic_base, http_client)?;
        if is_oauth {
            let factory = crate::interface::shared::make_provider_factory(
                "anthropic",
                anthropic_base,
                http_client.clone(),
            );
            provider_list.push(Arc::new(RefreshableProvider::new(RefreshableConfig {
                inner,
                store: store_arc.clone(),
                provider_name: "anthropic".to_string(),
                refresh_fn: refresh_fn.clone(),
                factory,
            })));
        } else {
            provider_list.push(inner);
        }
    }

    if provider_list.is_empty() {
        return Err(
            "no LLM providers configured (set an API key or run 'quecto auth login')".to_string(),
        );
    }

    Ok(Arc::new(ProviderRouter::new(provider_list)))
}

/// Build a single provider from name, key, and base URL.
fn build_single_provider(
    name: &str,
    api_key: &str,
    api_base: &Option<String>,
    http_client: &reqwest::Client,
) -> Result<Arc<dyn LlmProvider>, String> {
    if name == "openai" {
        let account_id = crate::infrastructure::auth::oauth::extract_openai_account_id(api_key);
        if let Some(acct) = account_id {
            return Ok(providers::create_codex_provider_with_client(
                api_key.to_string(),
                acct,
                http_client.clone(),
            ));
        }
    }
    let base = api_base.clone();
    providers::create_provider_with_client(name, api_key.to_string(), base, http_client.clone())
        .map_err(|e| format!("{} provider configuration error: {}", name, e))
}
