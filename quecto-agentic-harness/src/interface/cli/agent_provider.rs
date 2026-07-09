//! Provider construction for the agent CLI.

use std::collections::HashSet;
use std::sync::Arc;

use crate::domain::provider::LlmProvider;
use crate::infrastructure::auth::credential_store::CredentialStore;
use crate::infrastructure::config::Config;
use crate::infrastructure::providers;
use crate::infrastructure::providers::refreshable::{RefreshableConfig, RefreshableProvider};
use crate::infrastructure::providers::retry::{RetryConfig, RetryingProvider};
use crate::infrastructure::providers::router::ProviderRouter;

const MAX_OPENAI_COMPATIBLE_ENDPOINTS: usize = 32;

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

    let mut provider_list: Vec<Arc<dyn crate::domain::provider::LlmProvider>> = Vec::new();
    let store_arc = Arc::new(CredentialStore::new(base_dir));
    let refresh_fn = crate::interface::shared::make_oauth_refresh_fn();

    // Built-in providers are explicit by billing/auth mode. We deliberately do
    // not resolve a single `openai`/`anthropic` slot by precedence because that
    // can silently switch a request between monthly-plan OAuth and token-billed
    // API-key auth. Users select `openai-api`, `openai-oauth`, `anthropic-api`,
    // or `anthropic-oauth` explicitly (or define their own keys in models.json).
    let openai_base = non_empty(config.providers.openai.api_base.clone());
    let openai_api_key = if !config.providers.openai.api_key.is_empty() {
        config.providers.openai.api_key.clone()
    } else {
        store
            .get("openai")
            .ok()
            .flatten()
            .filter(|c| {
                c.method == crate::infrastructure::auth::credential_store::AuthMethod::Token
            })
            .filter(|c| !c.is_expired())
            .map(|c| c.token)
            .unwrap_or_default()
    };
    if !openai_api_key.is_empty() {
        provider_list.push(
            providers::create_named_openai_provider_with_client(
                "openai-api",
                openai_api_key,
                openai_base.clone(),
                http_client.clone(),
                false,
            )
            .map_err(|e| format!("openai-api provider configuration error: {}", e))?,
        );
    }
    if let Some(openai_oauth_cred) =
        store.get("openai").ok().flatten().filter(|c| {
            c.method == crate::infrastructure::auth::credential_store::AuthMethod::OAuth
        })
    {
        // #811: construct from the stored (possibly stale) token — no eager
        // network refresh on the pre-announce startup path. RefreshableProvider
        // refreshes lazily on a 401 at first real request, after socket announce.
        let openai_oauth_key = openai_oauth_cred.token;
        if !openai_oauth_key.is_empty() {
            let inner = build_single_provider(
                "openai",
                &openai_oauth_key,
                &openai_base,
                http_client,
                false,
            )?;
            let factory = crate::interface::shared::make_provider_factory(
                "openai",
                openai_base.clone(),
                http_client.clone(),
            );
            provider_list.push(Arc::new(RefreshableProvider::new(RefreshableConfig {
                inner,
                store: store_arc.clone(),
                provider_name: "openai-oauth".to_string(),
                credential_provider: "openai".to_string(),
                refresh_fn: refresh_fn.clone(),
                factory,
            })));
        }
    }

    let anthropic_base = non_empty(config.providers.anthropic.api_base.clone());
    let anthropic_api_key = if !config.providers.anthropic.api_key.is_empty() {
        config.providers.anthropic.api_key.clone()
    } else {
        store
            .get("anthropic")
            .ok()
            .flatten()
            .filter(|c| {
                c.method == crate::infrastructure::auth::credential_store::AuthMethod::Token
            })
            .filter(|c| !c.is_expired())
            .map(|c| c.token)
            .unwrap_or_default()
    };
    if !anthropic_api_key.is_empty() {
        provider_list.push(
            providers::create_anthropic_compatible_provider(
                "anthropic-api",
                anthropic_api_key,
                anthropic_base.clone(),
                false,
                http_client.clone(),
            )
            .map_err(|e| format!("anthropic-api provider configuration error: {}", e))?,
        );
        #[cfg(feature = "test-support")]
        if mock_llm_bare_anthropic_alias_enabled(&anthropic_base) {
            provider_list.push(
                providers::create_provider_with_client(
                    "anthropic",
                    config.providers.anthropic.api_key.clone(),
                    anthropic_base.clone(),
                    http_client.clone(),
                )
                .map_err(|e| format!("anthropic provider configuration error: {}", e))?,
            );
        }
    }
    if let Some(anthropic_oauth_cred) =
        store.get("anthropic").ok().flatten().filter(|c| {
            c.method == crate::infrastructure::auth::credential_store::AuthMethod::OAuth
        })
    {
        // #811: construct from the stored (possibly stale) token — no eager
        // network refresh on the pre-announce startup path. RefreshableProvider
        // refreshes lazily on a 401 at first real request, after socket announce.
        let anthropic_oauth_key = anthropic_oauth_cred.token;
        if !anthropic_oauth_key.is_empty() {
            let inner = providers::create_anthropic_compatible_provider(
                "anthropic-oauth",
                anthropic_oauth_key,
                anthropic_base.clone(),
                false,
                http_client.clone(),
            )
            .map_err(|e| format!("anthropic-oauth provider configuration error: {}", e))?;
            let factory = registry_provider_factory(
                crate::infrastructure::model_registry::ProviderApi::AnthropicMessages,
                "anthropic-oauth".to_string(),
                anthropic_base.clone(),
                false,
                http_client.clone(),
            );
            provider_list.push(Arc::new(RefreshableProvider::new(RefreshableConfig {
                inner,
                store: store_arc.clone(),
                provider_name: "anthropic-oauth".to_string(),
                credential_provider: "anthropic".to_string(),
                refresh_fn: refresh_fn.clone(),
                factory,
            })));
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
    // Build at most one provider per distinct registry provider key. Each key
    // carries its own wire protocol (`api`) and explicit auth mode; we never
    // silently switch a vendor between OAuth and API-key billing.
    let mut seen_registry_prefixes = HashSet::new();
    for model in model_registry.models() {
        let canonical_prefix = model.provider.to_ascii_lowercase();
        if seen_registry_prefixes.contains(&canonical_prefix)
            || provider_list
                .iter()
                .any(|p| p.name().eq_ignore_ascii_case(&model.provider))
        {
            continue;
        }
        let Some(provider) =
            build_registry_provider(model, base_dir, &store_arc, &refresh_fn, http_client)?
        else {
            continue;
        };
        seen_registry_prefixes.insert(canonical_prefix.clone());
        // Reserve the prefix so an openai_compatible endpoint cannot collide.
        custom_prefixes.insert(canonical_prefix);
        provider_list.push(provider);
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

    // Wrap the router in the retry decorator so transient/retryable provider
    // errors (429 / 5xx-529 / network) are retried with bounded backoff + jitter
    // (honouring Retry-After) before the turn fails; Client/Auth/Cancelled pass
    // straight through (#931). Composed outside refreshable so a refreshed-token
    // retry still benefits from transient-error retries.
    let router: Arc<dyn LlmProvider> = Arc::new(ProviderRouter::new(provider_list));
    Ok(Arc::new(RetryingProvider::new(
        router,
        RetryConfig::default(),
    )))
}

#[cfg(feature = "test-support")]
fn mock_llm_bare_anthropic_alias_enabled(api_base: &Option<String>) -> bool {
    std::env::var("QUECTO_TAG").ok().as_deref() == Some("mock-llm")
        && api_base.as_deref().is_some_and(|base| {
            base.starts_with("http://127.0.0.1:") || base.starts_with("http://localhost:")
        })
}

fn registry_provider_factory(
    provider_api: crate::infrastructure::model_registry::ProviderApi,
    provider_prefix: String,
    base: Option<String>,
    allow_remote_http: bool,
    client: reqwest::Client,
) -> crate::infrastructure::providers::refreshable::ProviderFactory {
    use crate::infrastructure::model_registry::ProviderApi;
    Arc::new(move |new_token: &str| -> Arc<dyn LlmProvider> {
        match provider_api {
            ProviderApi::OpenAiCompletions => {
                let base = base
                    .clone()
                    .expect("OpenAI-compatible registry provider base should be validated");
                providers::create_openai_compatible_provider(
                    &provider_prefix,
                    new_token.to_string(),
                    base,
                    allow_remote_http,
                    client.clone(),
                )
                .expect("refreshed OpenAI-compatible registry provider should rebuild")
            }
            ProviderApi::AnthropicMessages => providers::create_anthropic_compatible_provider(
                &provider_prefix,
                new_token.to_string(),
                base.clone(),
                allow_remote_http,
                client.clone(),
            )
            .expect("refreshed Anthropic registry provider should rebuild"),
            ProviderApi::GoogleGenerativeAi => unreachable!("validated before factory creation"),
        }
    })
}

fn oauth_registry_base_url(
    model: &crate::infrastructure::model_registry::ModelRecord,
    oauth_provider: &str,
) -> Result<Option<String>, String> {
    use crate::infrastructure::model_registry::ProviderApi;

    let configured = model.base_url.as_ref().filter(|b| !b.trim().is_empty());
    match (model.api, oauth_provider) {
        (ProviderApi::OpenAiCompletions, "openai") => validate_oauth_base_url(
            &model.provider,
            oauth_provider,
            configured,
            "https://api.openai.com/v1",
        )
        .map(Some),
        (ProviderApi::AnthropicMessages, "anthropic") => validate_oauth_base_url(
            &model.provider,
            oauth_provider,
            configured,
            "https://api.anthropic.com",
        )
        .map(Some),
        (ProviderApi::OpenAiCompletions | ProviderApi::AnthropicMessages, _) => Err(format!(
            "models.json provider '{}' uses oauthProvider '{}' with incompatible api {:?}",
            model.provider, oauth_provider, model.api
        )),
        (ProviderApi::GoogleGenerativeAi, _) => Ok(configured.cloned()),
    }
}

fn validate_oauth_base_url(
    provider_key: &str,
    oauth_provider: &str,
    configured: Option<&String>,
    canonical: &str,
) -> Result<String, String> {
    let Some(configured) = configured else {
        return Ok(canonical.to_string());
    };
    let configured_url = reqwest::Url::parse(configured).map_err(|e| {
        format!(
            "models.json provider '{}' has invalid OAuth baseUrl '{}': {}",
            provider_key, configured, e
        )
    })?;
    let canonical_url = reqwest::Url::parse(canonical).expect("canonical OAuth base URL is valid");
    if configured_url.scheme() == canonical_url.scheme()
        && configured_url.host_str() == canonical_url.host_str()
        && configured_url.port_or_known_default() == canonical_url.port_or_known_default()
    {
        return Ok(configured.clone());
    }
    Err(format!(
        "models.json provider '{}' uses oauth auth for '{}' but baseUrl '{}' is not the canonical OAuth host '{}'",
        provider_key, oauth_provider, configured, canonical
    ))
}

fn build_registry_provider(
    model: &crate::infrastructure::model_registry::ModelRecord,
    _base_dir: &std::path::Path,
    store: &Arc<CredentialStore>,
    refresh_fn: &crate::infrastructure::providers::refreshable::RefreshFn,
    http_client: &reqwest::Client,
) -> Result<Option<Arc<dyn LlmProvider>>, String> {
    use crate::infrastructure::model_registry::{AuthMode, ProviderApi};

    let mut api_base = model.base_url.clone();
    let auth_key = match model.auth {
        AuthMode::ApiKey => {
            let Some(key) = model.api_key.as_ref().filter(|k| !k.is_empty()) else {
                return Ok(None);
            };
            key.clone()
        }
        AuthMode::OAuth => {
            let oauth_provider = model.oauth_provider.as_deref().ok_or_else(|| {
                format!(
                    "models.json provider '{}' uses oauth auth but is missing oauthProvider",
                    model.provider
                )
            })?;
            if crate::infrastructure::auth::oauth::OAuthConfig::for_provider(oauth_provider)
                .is_none()
            {
                return Err(format!(
                    "models.json provider '{}' references oauthProvider '{}' which is not a kernel OAuth provider",
                    model.provider, oauth_provider
                ));
            }
            let Some(cred) = store.get(oauth_provider).map_err(|e| e.to_string())? else {
                return Ok(None);
            };
            if cred.method != crate::infrastructure::auth::credential_store::AuthMethod::OAuth {
                return Ok(None);
            }
            if cred.token.is_empty() {
                return Ok(None);
            }
            api_base = oauth_registry_base_url(model, oauth_provider)?;
            // #811: use the stored (possibly stale) token; no eager network
            // refresh. RefreshableProvider refreshes lazily on 401 at first use.
            cred.token
        }
    };

    let inner: Arc<dyn LlmProvider> = match model.api {
        ProviderApi::OpenAiCompletions => {
            let Some(base) = api_base.clone().filter(|b| !b.trim().is_empty()) else {
                return Ok(None);
            };
            providers::create_openai_compatible_provider(
                &model.provider,
                auth_key.clone(),
                base,
                model.allow_remote_http,
                http_client.clone(),
            )
            .map_err(|e| format!("models.json provider configuration error: {}", e))?
        }
        ProviderApi::AnthropicMessages => providers::create_anthropic_compatible_provider(
            &model.provider,
            auth_key.clone(),
            api_base.clone(),
            model.allow_remote_http,
            http_client.clone(),
        )
        .map_err(|e| format!("models.json provider configuration error: {}", e))?,
        ProviderApi::GoogleGenerativeAi => {
            return Err(format!(
                "models.json provider '{}' uses google-generative-ai, but that wire protocol is not implemented yet",
                model.provider
            ));
        }
    };

    if model.auth == AuthMode::OAuth {
        let oauth_provider = model.oauth_provider.clone().expect("validated above");
        let factory = registry_provider_factory(
            model.api,
            model.provider.clone(),
            api_base.clone(),
            model.allow_remote_http,
            http_client.clone(),
        );
        return Ok(Some(Arc::new(RefreshableProvider::new(
            RefreshableConfig {
                inner,
                store: store.clone(),
                provider_name: model.provider.clone(),
                credential_provider: oauth_provider,
                refresh_fn: refresh_fn.clone(),
                factory,
            },
        ))));
    }

    Ok(Some(inner))
}

fn non_empty(value: String) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
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
                api_base.clone(),
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
