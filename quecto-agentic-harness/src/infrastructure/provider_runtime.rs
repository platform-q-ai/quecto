use crate::domain::provider::LlmProvider;
use crate::infrastructure::auth::credential_store::CredentialStore;
use crate::infrastructure::catalogue_registry::record_to_descriptor_with_credential;
use crate::infrastructure::config::Config;
use crate::infrastructure::providers;
use crate::infrastructure::providers::refreshable::{RefreshableConfig, RefreshableProvider};
use crate::infrastructure::providers::retry::{RetryConfig, RetryingProvider};
use crate::infrastructure::providers::router::ProviderRouter;
use std::collections::HashSet;
use std::sync::Arc;
const MAX_OPENAI_COMPATIBLE_ENDPOINTS: usize = 32;
#[derive(Debug, Default, Clone, Copy)]
pub struct InfrastructureProviderRuntimeFactory;
pub struct ProviderRuntimeInputs<'a> {
    pub base_dir: &'a std::path::Path,
    pub http_client: &'a reqwest::Client,
}
impl<'a> crate::application::ports::ProviderRuntimeFactory<Config, ProviderRuntimeInputs<'a>>
    for InfrastructureProviderRuntimeFactory
{
    fn compose_runtime(
        &self,
        config: &Config,
        runtime_inputs: &ProviderRuntimeInputs<'a>,
    ) -> Result<Arc<dyn LlmProvider>, String> {
        build_agent_provider(config, runtime_inputs.base_dir, runtime_inputs.http_client)
    }
}
#[path = "provider_runtime_credentials.rs"]
pub(crate) mod credentials;

use credentials::{
    explicit_endpoint_owns_registry_route, registry_api_key, registry_model_credential_available,
    registry_provider_can_construct,
};

pub(crate) fn build_agent_provider(
    config: &Config,
    base_dir: &std::path::Path,
    http_client: &reqwest::Client,
) -> Result<Arc<dyn LlmProvider>, String> {
    compose_agent_provider(config, base_dir, http_client)
}
fn compose_agent_provider(
    config: &Config,
    base_dir: &std::path::Path,
    http_client: &reqwest::Client,
) -> Result<Arc<dyn LlmProvider>, String> {
    let store = CredentialStore::new(base_dir);
    // One read of the credential store serves the whole composition.
    let credentials = credentials::CredentialSnapshot::load(&store)?;
    let mut provider_list: Vec<Arc<dyn crate::domain::provider::LlmProvider>> = Vec::new();
    let store_arc = Arc::new(CredentialStore::new(base_dir));
    let refresh_fn = crate::infrastructure::oauth_runtime::make_oauth_refresh_fn();
    let model_registry = crate::infrastructure::model_registry::ModelRegistry::load_from_path(
        &base_dir.join("models.json"),
    )
    .map_err(|e| e.to_string())?;
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
    let has_openai_api_key = !openai_api_key.is_empty();
    if has_openai_api_key {
        provider_list.push(
            providers::create_named_openai_provider_with_client(
                "openai-api",
                openai_api_key,
                openai_base.clone(),
                http_client.clone(),
                false,
                model_registry
                    .models()
                    .iter()
                    .filter(|m| m.provider == "openai-api" && m.reasoning)
                    .map(|m| m.id.clone())
                    .collect(),
            )
            .map_err(|e| format!("openai-api provider configuration error: {}", e))?,
        );
    }
    if let Some(openai_oauth_cred) =
        store.get("openai").ok().flatten().filter(|c| {
            c.method == crate::infrastructure::auth::credential_store::AuthMethod::OAuth
        })
    {
        let openai_oauth_key = openai_oauth_cred.token;
        if !openai_oauth_key.is_empty() {
            let inner = build_single_provider(
                "openai",
                &openai_oauth_key,
                &openai_base,
                http_client,
                config.providers.openai.disable_codex_routing,
            )?;
            let factory =
                crate::infrastructure::oauth_runtime::make_provider_factory_with_codex_routing(
                    "openai",
                    openai_base.clone(),
                    http_client.clone(),
                    config.providers.openai.disable_codex_routing,
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
    let has_anthropic_api_key = !anthropic_api_key.is_empty();
    if has_anthropic_api_key {
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
    validate_endpoint_count(config)?;
    let mut custom_prefixes = HashSet::new();
    let mut seen_registry_prefixes = HashSet::new();
    let builtin_registry_prefixes: HashSet<String> =
        crate::infrastructure::model_registry::ModelRegistry::builtin()
            .models()
            .iter()
            .map(|model| model.provider.to_ascii_lowercase())
            .collect();
    custom_prefixes.extend(
        providers::RESERVED_PROVIDER_PREFIXES
            .iter()
            .map(|p| p.to_ascii_lowercase()),
    );
    let canonical_registry_prefixes = canonical_registry_prefix_owners(model_registry.models());
    // Endpoints without a key are skipped when providers are constructed, so
    // they must not mark catalogue routes runnable either.
    let configured_endpoint_prefixes: HashSet<String> = config
        .providers
        .openai_compatible
        .endpoints
        .iter()
        .filter(|endpoint| !endpoint.api_key.trim().is_empty())
        .map(|endpoint| endpoint.prefix.trim().to_ascii_lowercase())
        .collect();
    let mut constructible_registry_prefixes = configured_endpoint_prefixes.clone();
    if has_openai_api_key {
        constructible_registry_prefixes.insert("openai-api".to_string());
    }
    if has_anthropic_api_key {
        constructible_registry_prefixes.insert("anthropic-api".to_string());
    }
    for model in model_registry.models() {
        let has_existing_provider = provider_list
            .iter()
            .any(|provider| provider.name().eq_ignore_ascii_case(&model.provider))
            || configured_endpoint_prefixes.contains(&model.provider.to_ascii_lowercase());
        if canonical_registry_prefixes.contains(&model.provider)
            && (has_existing_provider
                || registry_provider_can_construct(model, &credentials, config)?)
        {
            constructible_registry_prefixes.insert(model.provider.to_ascii_lowercase());
        }
    }
    let runtime_model_descriptors = catalogue_descriptors(&DescriptorInputs {
        model_registry: &model_registry,
        credentials: &credentials,
        config,
        canonical_registry_prefixes: &canonical_registry_prefixes,
        configured_endpoint_prefixes: &configured_endpoint_prefixes,
        constructible_registry_prefixes: &constructible_registry_prefixes,
        has_openai_api_key,
        has_anthropic_api_key,
    })?;
    for model in model_registry.models() {
        let canonical_prefix = model.provider.to_ascii_lowercase();
        if !canonical_registry_prefixes.contains(&model.provider)
            || provider_list
                .iter()
                .any(|p| p.name().eq_ignore_ascii_case(&model.provider))
            || seen_registry_prefixes.contains(&canonical_prefix)
        {
            continue;
        }
        seen_registry_prefixes.insert(canonical_prefix.clone());
        if explicit_endpoint_owns_registry_route(
            model,
            &configured_endpoint_prefixes,
            &credentials,
            config,
        )? {
            continue;
        }
        if !builtin_registry_prefixes.contains(&canonical_prefix) {
            custom_prefixes.insert(canonical_prefix.clone());
        }
        if matches!(
            model.api,
            crate::infrastructure::model_registry::ProviderApi::GoogleGenerativeAi
        ) {
            continue;
        }
        let Some(provider) = build_registry_provider(
            model,
            base_dir,
            &RegistryProviderContext {
                store: &store_arc,
                credentials: &credentials,
                refresh_fn: &refresh_fn,
                http_client,
                config,
            },
        )?
        else {
            continue;
        };
        // A constructed provider owns its prefix, built-in or not, so a later
        // endpoint declaring the same prefix is reported as the duplicate it is
        // rather than producing two providers of one name.
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
    ensure_providers_configured(&provider_list)?;
    let router: Arc<dyn LlmProvider> = Arc::new(ProviderRouter::try_with_model_descriptors(
        provider_list,
        runtime_model_descriptors,
    )?);
    Ok(Arc::new(RetryingProvider::new(
        router,
        RetryConfig::default(),
    )))
}
/// What the catalogue projection of one composition needs to derive each
/// record's availability.
struct DescriptorInputs<'a> {
    model_registry: &'a crate::infrastructure::model_registry::ModelRegistry,
    credentials: &'a credentials::CredentialSnapshot,
    config: &'a Config,
    canonical_registry_prefixes: &'a HashSet<String>,
    configured_endpoint_prefixes: &'a HashSet<String>,
    constructible_registry_prefixes: &'a HashSet<String>,
    has_openai_api_key: bool,
    has_anthropic_api_key: bool,
}

/// Project the registry into domain descriptors, recording for each entry why
/// the runtime could not serve it.
fn catalogue_descriptors(
    inputs: &DescriptorInputs<'_>,
) -> Result<Vec<crate::domain::catalogue::ModelDescriptor>, String> {
    let mut runtime_model_descriptors = Vec::new();
    for model in inputs.model_registry.models() {
        let credential_available =
            registry_model_credential_available(model, inputs.credentials, inputs.config)?;
        if let Some(mut descriptor) =
            record_to_descriptor_with_credential(model, Some(credential_available))?
        {
            let canonical_provider = model.provider.to_ascii_lowercase();
            let is_canonical_owner = inputs.canonical_registry_prefixes.contains(&model.provider);
            let has_direct_runtime = is_canonical_owner
                && ((canonical_provider == "openai-api" && inputs.has_openai_api_key)
                    || (canonical_provider == "anthropic-api" && inputs.has_anthropic_api_key)
                    || inputs
                        .configured_endpoint_prefixes
                        .contains(&canonical_provider)
                    || inputs
                        .constructible_registry_prefixes
                        .contains(&canonical_provider));
            if !is_canonical_owner || !has_direct_runtime {
                // Keep the reasons derived from the catalogue entry (an
                // unimplemented transport, a missing credential) and add why the
                // runtime skipped it, so availability stays a complete account.
                let mut reasons = descriptor.availability.reasons().to_vec();
                let skipped = crate::domain::catalogue::UnavailableReason::InvalidConfiguration(
                    "provider skipped during runtime construction".to_string(),
                );
                if !reasons.contains(&skipped) {
                    reasons.push(skipped);
                }
                descriptor.availability =
                    crate::domain::catalogue::Availability::KnownButUnavailable { reasons };
            }
            runtime_model_descriptors.push(descriptor);
        }
    }
    Ok(runtime_model_descriptors)
}

fn ensure_providers_configured(providers: &[Arc<dyn LlmProvider>]) -> Result<(), String> {
    if providers.is_empty() {
        return Err(
            "no LLM providers configured (set an API key or run 'quecto auth login')".to_string(),
        );
    }
    Ok(())
}

fn validate_endpoint_count(config: &Config) -> Result<(), String> {
    let count = config.providers.openai_compatible.endpoints.len();
    if count > MAX_OPENAI_COMPATIBLE_ENDPOINTS {
        return Err(format!(
            "openai_compatible configures {count} endpoints, exceeding the maximum of {MAX_OPENAI_COMPATIBLE_ENDPOINTS}"
        ));
    }
    Ok(())
}

fn canonical_registry_prefix_owners<'a>(
    models: impl IntoIterator<Item = &'a crate::infrastructure::model_registry::ModelRecord>,
) -> HashSet<String> {
    let mut owners_by_canonical = std::collections::BTreeMap::new();
    for model in models {
        owners_by_canonical
            .entry(model.provider.to_ascii_lowercase())
            .and_modify(|owner: &mut String| {
                if model.provider < *owner {
                    *owner = model.provider.clone();
                }
            })
            .or_insert_with(|| model.provider.clone());
    }
    owners_by_canonical.into_values().collect()
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
        (ProviderApi::OpenAiCompletions, "xai") => validate_oauth_base_url(
            &model.provider,
            oauth_provider,
            configured,
            "https://api.x.ai/v1",
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
            provider_key,
            sanitize_url_for_error(configured),
            e
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
        provider_key,
        oauth_provider,
        sanitize_url_for_error(configured),
        canonical
    ))
}
fn sanitize_url_for_error(raw: &str) -> String {
    match reqwest::Url::parse(raw) {
        Ok(mut url) => {
            let _ = url.set_username("");
            let _ = url.set_password(None);
            url.set_query(None);
            url.set_fragment(None);
            url.to_string()
        }
        Err(_) => "<invalid url>".to_string(),
    }
}
/// Everything a registry record needs to become a concrete provider, resolved
/// once per composition rather than per record.
pub(crate) struct RegistryProviderContext<'a> {
    pub(crate) store: &'a Arc<CredentialStore>,
    pub(crate) credentials: &'a credentials::CredentialSnapshot,
    pub(crate) refresh_fn: &'a crate::infrastructure::providers::refreshable::RefreshFn,
    pub(crate) http_client: &'a reqwest::Client,
    pub(crate) config: &'a Config,
}

fn build_registry_provider(
    model: &crate::infrastructure::model_registry::ModelRecord,
    _base_dir: &std::path::Path,
    ctx: &RegistryProviderContext<'_>,
) -> Result<Option<Arc<dyn LlmProvider>>, String> {
    let store = ctx.store;
    let credentials = ctx.credentials;
    let refresh_fn = ctx.refresh_fn;
    let http_client = ctx.http_client;
    let config = ctx.config;
    use crate::infrastructure::model_registry::{AuthMode, ProviderApi};

    let mut api_base = model.base_url.clone();
    let auth_key = match model.auth {
        AuthMode::ApiKey => match registry_api_key(model, credentials, config)? {
            Some(key) => key,
            None => return Ok(None),
        },
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
        ProviderApi::GoogleGenerativeAi => return Ok(None),
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
                store: (*store).clone(),
                provider_name: model.provider.clone(),
                credential_provider: oauth_provider,
                refresh_fn: (*refresh_fn).clone(),
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
            return providers::create_codex_provider_with_client(
                api_key.to_string(),
                acct,
                api_base.clone(),
                http_client.clone(),
            )
            .map_err(|e| format!("openai provider configuration error: {}", e));
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

#[cfg(test)]
#[path = "../interface/cli/agent_provider_cov_tests.rs"]
mod agent_provider_cov_tests;

#[cfg(test)]
#[path = "../interface/cli/agent_provider_catalogue_tests.rs"]
mod agent_provider_catalogue_tests;
#[cfg(test)]
#[path = "../interface/cli/agent_provider_cycle4_tests.rs"]
mod agent_provider_cycle4_tests;
#[cfg(test)]
#[path = "../interface/cli/agent_provider_xai_tests.rs"]
mod agent_provider_xai_tests;
