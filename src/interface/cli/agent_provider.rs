//! Provider construction for the agent CLI.

use std::collections::HashSet;
use std::sync::Arc;

use crate::domain::provider::LlmProvider;
use crate::infrastructure::auth::credential_store::CredentialStore;
use crate::infrastructure::config::Config;
use crate::infrastructure::providers;
use crate::infrastructure::providers::refreshable::{RefreshableConfig, RefreshableProvider};
use crate::infrastructure::providers::router::ProviderRouter;

const MAX_OPENAI_COMPATIBLE_ENDPOINTS: usize = 32;

fn refresh_runtime(
    rt: &mut Option<tokio::runtime::Runtime>,
) -> Result<&tokio::runtime::Runtime, String> {
    if rt.is_none() {
        *rt = Some(
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| format!("failed to create runtime for token refresh: {}", e))?,
        );
    }
    Ok(rt.as_ref().expect("runtime was just initialized"))
}

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

    let mut refresh_rt: Option<tokio::runtime::Runtime> = None;

    let mut provider_list: Vec<Arc<dyn crate::domain::provider::LlmProvider>> = Vec::new();
    let store_arc = Arc::new(CredentialStore::new(base_dir));
    let refresh_fn = crate::interface::shared::make_oauth_refresh_fn();

    // Try OpenAI (with auto-refresh for expired OAuth tokens unless explicitly disabled).
    // `disable_codex_routing` is an escape hatch for users who keep an OpenAI
    // OAuth credential but intentionally point the built-in `openai` slot at a
    // custom OpenAI-compatible endpoint.
    let openai_key = if config.providers.openai.disable_codex_routing {
        config.providers.openai.api_key.clone()
    } else {
        let has_stored_openai = store.get("openai").ok().flatten().is_some();
        if config.providers.openai.api_key.is_empty() && !has_stored_openai {
            String::new()
        } else {
            crate::interface::shared::resolve_api_key_with_refresh(
                &config.providers.openai.api_key,
                &store,
                "openai",
                refresh_runtime(&mut refresh_rt)?,
            )
        }
    };
    if !openai_key.is_empty() {
        let is_oauth = !config.providers.openai.disable_codex_routing
            && store.get("openai").ok().flatten().is_some_and(|c| {
                c.method == crate::infrastructure::auth::credential_store::AuthMethod::OAuth
            });
        let openai_base = if config.providers.openai.api_base.is_empty() {
            None
        } else {
            Some(config.providers.openai.api_base.clone())
        };
        let inner = build_single_provider(
            "openai",
            &openai_key,
            &openai_base,
            http_client,
            config.providers.openai.disable_codex_routing,
        )?;
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
    let anthropic_key = {
        let has_stored_anthropic = store.get("anthropic").ok().flatten().is_some();
        if config.providers.anthropic.api_key.is_empty() && !has_stored_anthropic {
            String::new()
        } else {
            crate::interface::shared::resolve_api_key_with_refresh(
                &config.providers.anthropic.api_key,
                &store,
                "anthropic",
                refresh_runtime(&mut refresh_rt)?,
            )
        }
    };
    if !anthropic_key.is_empty() {
        let is_oauth = store.get("anthropic").ok().flatten().is_some_and(|c| {
            c.method == crate::infrastructure::auth::credential_store::AuthMethod::OAuth
        });
        let anthropic_base = if config.providers.anthropic.api_base.is_empty() {
            None
        } else {
            Some(config.providers.anthropic.api_base.clone())
        };
        let inner = build_single_provider(
            "anthropic",
            &anthropic_key,
            &anthropic_base,
            http_client,
            false,
        )?;
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

    if config.providers.openai_compatible.endpoints.len() > MAX_OPENAI_COMPATIBLE_ENDPOINTS {
        return Err(format!(
            "openai_compatible configures {} endpoints, exceeding the maximum of {}",
            config.providers.openai_compatible.endpoints.len(),
            MAX_OPENAI_COMPATIBLE_ENDPOINTS
        ));
    }
    let mut custom_prefixes = HashSet::new();
    let model_registry = crate::infrastructure::model_registry::ModelRegistry::load_from_path(
        &base_dir.join("models.json"),
    )
    .map_err(|e| e.to_string())?;
    for model in model_registry.models() {
        let Some(api_key) = model.api_key.as_ref().filter(|k| !k.is_empty()) else {
            continue;
        };
        let Some(api_base) = model.base_url.as_ref().filter(|b| !b.trim().is_empty()) else {
            continue;
        };
        let canonical_prefix = model.provider.to_ascii_lowercase();
        if !custom_prefixes.insert(canonical_prefix) {
            continue;
        }
        if matches!(
            model.api,
            crate::infrastructure::model_registry::ProviderApi::OpenAiCompletions
        ) {
            let provider = providers::create_openai_compatible_provider(
                &model.provider,
                api_key.clone(),
                api_base.clone(),
                model.allow_remote_http,
                http_client.clone(),
            )
            .map_err(|e| format!("models.json provider configuration error: {}", e))?;
            provider_list.push(provider);
        }
    }
    for endpoint in &config.providers.openai_compatible.endpoints {
        if endpoint.api_key.is_empty() {
            continue;
        }
        let prefix = endpoint.prefix.trim();
        if prefix.is_empty() || endpoint.api_base.trim().is_empty() {
            return Err("openai_compatible endpoint requires prefix and api_base".to_string());
        }
        let canonical_prefix = prefix.to_ascii_lowercase();
        if !custom_prefixes.insert(canonical_prefix) {
            return Err(format!(
                "duplicate openai_compatible/provider prefix '{}'",
                prefix
            ));
        }
        let provider = providers::create_openai_compatible_provider(
            &endpoint.prefix,
            endpoint.api_key.clone(),
            endpoint.api_base.clone(),
            endpoint.allow_remote_http,
            http_client.clone(),
        )
        .map_err(|e| format!("openai_compatible provider configuration error: {}", e))?;
        provider_list.push(provider);
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
    disable_codex_routing: bool,
) -> Result<Arc<dyn LlmProvider>, String> {
    if name == "openai" && !disable_codex_routing {
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
    if name == "openai" && disable_codex_routing {
        return providers::create_openai_provider_with_client(
            api_key.to_string(),
            base,
            http_client.clone(),
            false,
        )
        .map_err(|e| format!("{} provider configuration error: {}", name, e));
    }
    providers::create_provider_with_client(name, api_key.to_string(), base, http_client.clone())
        .map_err(|e| format!("{} provider configuration error: {}", name, e))
}
