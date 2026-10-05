//! Concrete provider-runtime composition (epic #1193, slice 3).
//!
//! Implements the application's `ProviderRuntimeFactory` port: constructs the
//! concrete execution adapters (OpenAI/Anthropic/compatible endpoints), applies
//! OAuth wrapping, retry decoration, and router composition. This orchestration
//! previously lived in the interface; `composition::runtime` now wires
//! [`AgentRuntimeInputs`] and invokes the compose-provider-runtime use case.

use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use crate::application::ports::{
    AdmissionBindingDiagnostic, ProviderRuntimeFactory, ProviderRuntimeOutcome,
};
use crate::infrastructure::providers::refreshable::{ProviderFactory, RefreshFn};

use crate::application::providers::ports::LlmProvider;
use crate::infrastructure::auth::credential_store::CredentialStore;
use crate::infrastructure::auth::provider_slots::{
    ANTHROPIC_API, ANTHROPIC_OAUTH, OPENAI_API, OPENAI_OAUTH, api_slot_key, oauth_slot_token,
};
use crate::infrastructure::config::Config;
use crate::infrastructure::providers;
use crate::infrastructure::providers::model_refusal::RefusalRecordingProvider;
use crate::infrastructure::providers::refreshable::{RefreshableConfig, RefreshableProvider};
use crate::infrastructure::providers::retry::{RetryConfig, RetryingProvider};
use crate::infrastructure::providers::router::ProviderRouter;

use super::provider_runtime_admission::AdmissionRuntimeContext;
use crate::infrastructure::providers::stream_idle::StreamIdle;
use crate::infrastructure::providers::{AttemptTransportBinding, ProviderBinding};

const MAX_OPENAI_COMPATIBLE_ENDPOINTS: usize = 32;

/// The non-config inputs one runtime composition needs. Entry points wire
/// these (the OAuth refresh/rebuild closures come from the interface layer's
/// credential-sync policy) and pass them through the compose use case.
pub struct AgentRuntimeInputs {
    pub base_dir: PathBuf,
    pub http_client: reqwest::Client,
    /// Refreshes an expired OAuth token mid-session (401 path).
    pub refresh_fn: RefreshFn,
    /// Rebuilds the `openai` OAuth provider (Codex-aware) after a refresh.
    pub openai_oauth_factory: ProviderFactory,
    /// The effective model registry (or the models.json parse error) from the
    /// same on-disk read that fed the catalogue resolve, so router and
    /// catalogue in one composed generation describe one on-disk state.
    pub model_registry: Result<crate::infrastructure::model_registry::ModelRegistry, String>,
}

/// Infrastructure implementation of the application's runtime-factory port:
/// the one place concrete providers, credential resolution, and router
/// orchestration happen.
#[derive(Debug, Default, Clone, Copy)]
pub struct AgentProviderRuntimeFactory;

impl ProviderRuntimeFactory<Config, AgentRuntimeInputs> for AgentProviderRuntimeFactory {
    fn compose_runtime(
        &self,
        config: &Config,
        runtime_inputs: &AgentRuntimeInputs,
    ) -> Result<Arc<dyn LlmProvider>, String> {
        compose_agent_provider(config, runtime_inputs)
    }

    fn compose_runtime_outcome(
        &self,
        config: &Config,
        runtime_inputs: &AgentRuntimeInputs,
    ) -> Result<ProviderRuntimeOutcome, String> {
        compose_agent_provider_inner_outcome(config, runtime_inputs, None)
    }
}

/// Build a ProviderRouter from config + credential store.
///
/// OAuth-backed providers are wrapped in [`RefreshableProvider`] so that
/// expired tokens are automatically refreshed mid-session on 401 (issue #255).
pub fn compose_agent_provider(
    config: &Config,
    inputs: &AgentRuntimeInputs,
) -> Result<Arc<dyn LlmProvider>, String> {
    compose_agent_provider_inner(config, inputs, None)
}

pub(crate) fn compose_agent_provider_inner(
    config: &Config,
    inputs: &AgentRuntimeInputs,
    admission: Option<&AdmissionRuntimeContext>,
) -> Result<Arc<dyn LlmProvider>, String> {
    compose_agent_provider_inner_outcome(config, inputs, admission).map(|outcome| outcome.provider)
}

pub(crate) fn compose_agent_provider_inner_outcome(
    config: &Config,
    inputs: &AgentRuntimeInputs,
    admission: Option<&AdmissionRuntimeContext>,
) -> Result<ProviderRuntimeOutcome, String> {
    let base_dir: &std::path::Path = &inputs.base_dir;
    let http_client = &inputs.http_client;
    let store = CredentialStore::new(base_dir);

    let mut provider_list: Vec<Arc<dyn crate::application::providers::ports::LlmProvider>> =
        Vec::new();
    let store_arc = Arc::new(CredentialStore::new(base_dir));
    let refresh_fn = inputs.refresh_fn.clone();

    // #1066: the endpoint router needs the *effective* registry (builtin +
    // ~/.quecto/models.json overrides) so user `reasoning` overrides steer
    // Responses-vs-Chat-Completions routing. Wired by the entry point from
    // the same models.json read that fed the catalogue resolve, so router and
    // catalogue never describe different on-disk states.
    let model_registry = inputs.model_registry.as_ref().map_err(Clone::clone)?;

    // Built-in providers are explicit by billing/auth mode. We deliberately do
    // not resolve a single `openai`/`anthropic` slot by precedence because that
    // can silently switch a request between monthly-plan OAuth and token-billed
    // API-key auth. Users select `openai-api`, `openai-oauth`, `anthropic-api`,
    // or `anthropic-oauth` explicitly (or define their own keys in models.json).
    let openai_base = non_empty(config.providers.openai.api_base.clone());
    let openai_idle = config
        .providers
        .openai
        .stream_limits
        .bounds("providers.openai")?;
    let anthropic = config.providers.anthropic.stream_limits;
    let anthropic_idle = anthropic.bounds("providers.anthropic")?;
    // The slot rules are shared with the catalogue's credential status
    // (`auth::provider_slots`, #2451), so the model selector offers exactly
    // the slots built here.
    if let Some(openai_api_key) = api_slot_key(
        &config.providers.openai.api_key,
        store.get("openai").ok().flatten(),
    ) {
        provider_list.push(
            providers::create_named_openai_provider_with_client_and_admission(
                OPENAI_API,
                openai_api_key,
                openai_base.clone(),
                providers::ProviderTransportContext {
                    client: http_client.clone(),
                    binding: bound(admission, OPENAI_API, openai_idle)?,
                },
                false,
                model_registry
                    .models()
                    .iter()
                    .filter(|m| m.provider == OPENAI_API && m.reasoning)
                    .map(|m| m.id.clone())
                    .collect(),
            )
            .map_err(|e| format!("openai-api provider configuration error: {}", e))?,
        );
    }
    // #811: construct from the stored (possibly stale) token — no eager
    // network refresh on the pre-announce startup path. RefreshableProvider
    // refreshes lazily on a 401 at first real request, after socket announce.
    if let Some(openai_oauth_key) = oauth_slot_token(store.get("openai").ok().flatten()) {
        let inner = build_single_provider_with_admission(
            "openai",
            &openai_oauth_key,
            &openai_base,
            http_client,
            false,
            bound(admission, OPENAI_OAUTH, openai_idle)?,
        )?;
        let factory = if admission.is_some() {
            let binding = bound(admission, OPENAI_OAUTH, openai_idle)?;
            let base = openai_base.clone();
            let client = http_client.clone();
            Arc::new(move |token: &str| {
                build_single_provider_with_admission(
                    "openai",
                    token,
                    &base,
                    &client,
                    false,
                    binding.clone(),
                )
                .expect("validated OpenAI OAuth provider should rebuild")
            }) as ProviderFactory
        } else {
            inputs.openai_oauth_factory.clone()
        };
        provider_list.push(Arc::new(RefreshableProvider::new(RefreshableConfig {
            inner,
            store: store_arc.clone(),
            provider_name: OPENAI_OAUTH.to_string(),
            credential_provider: "openai".to_string(),
            refresh_fn: refresh_fn.clone(),
            factory,
        })));
    }

    let anthropic_base = non_empty(config.providers.anthropic.api_base.clone());
    if let Some(anthropic_api_key) = api_slot_key(
        &config.providers.anthropic.api_key,
        store.get("anthropic").ok().flatten(),
    ) {
        provider_list.push(
            providers::create_anthropic_compatible_provider_and_admission(
                ANTHROPIC_API,
                anthropic_api_key,
                anthropic_base.clone(),
                false,
                http_client.clone(),
                bound(admission, ANTHROPIC_API, anthropic_idle)?,
            )
            .map_err(|e| format!("anthropic-api provider configuration error: {}", e))?,
        );
        #[cfg(feature = "test-support")]
        if mock_llm_bare_anthropic_alias_enabled(&anthropic_base) {
            provider_list.push(
                providers::create_provider_with_client_and_admission(
                    "anthropic",
                    config.providers.anthropic.api_key.clone(),
                    anthropic_base.clone(),
                    http_client.clone(),
                    bound(admission, "anthropic", anthropic_idle)?,
                )
                .map_err(|e| format!("anthropic provider configuration error: {}", e))?,
            );
        }
    }
    if let Some(anthropic_oauth_key) = oauth_slot_token(store.get("anthropic").ok().flatten()) {
        let inner = providers::create_anthropic_compatible_provider_and_admission(
            ANTHROPIC_OAUTH,
            anthropic_oauth_key,
            anthropic_base.clone(),
            false,
            http_client.clone(),
            bound(admission, ANTHROPIC_OAUTH, anthropic_idle)?,
        )
        .map_err(|e| format!("anthropic-oauth provider configuration error: {}", e))?;
        let factory = registry_provider_factory_with_admission(
            crate::infrastructure::model_registry::ProviderApi::AnthropicMessages,
            ANTHROPIC_OAUTH.to_string(),
            anthropic_base.clone(),
            false,
            http_client.clone(),
            bound(admission, ANTHROPIC_OAUTH, anthropic_idle)?,
        );
        provider_list.push(Arc::new(RefreshableProvider::new(RefreshableConfig {
            inner,
            store: store_arc.clone(),
            provider_name: ANTHROPIC_OAUTH.to_string(),
            credential_provider: "anthropic".to_string(),
            refresh_fn: refresh_fn.clone(),
            factory,
        })));
    }

    if config.providers.openai_compatible.endpoints.len() > MAX_OPENAI_COMPATIBLE_ENDPOINTS {
        return Err(format!(
            "openai_compatible configures {} endpoints, exceeding the maximum of {}",
            config.providers.openai_compatible.endpoints.len(),
            MAX_OPENAI_COMPATIBLE_ENDPOINTS
        ));
    }
    let mut custom_prefixes = HashSet::new();
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
        let Some(provider) = build_registry_provider_with_admission(
            model,
            base_dir,
            &store_arc,
            &refresh_fn,
            http_client,
            admission,
        )?
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
        let setting = format!("openai_compatible endpoint '{prefix}'");
        let idle = endpoint.stream_limits.bounds(&setting)?;
        let provider = providers::create_openai_compatible_provider_and_admission(
            &endpoint.prefix,
            endpoint.api_key.clone(),
            endpoint.api_base.clone(),
            endpoint.allow_remote_http,
            http_client.clone(),
            bound(admission, &endpoint.prefix, idle)?,
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
    let mut unbound_slots = BTreeSet::new();
    if let Some(context) = admission {
        for provider in &provider_list {
            if context.optional_binding(provider.name())?.is_none() {
                unbound_slots.insert(provider.name().trim().to_owned());
            }
        }
    }
    // #2435: a provider's definitive refusal of a model for this account
    // is recorded in the base directory's catalogue store, so the
    // catalogue stops offering it while the refusal is held.
    let router: Arc<dyn LlmProvider> = Arc::new(RefusalRecordingProvider::new(
        Arc::new(ProviderRouter::new(provider_list)),
        Arc::new(crate::infrastructure::catalogue_registry::snapshot_store_for(base_dir)),
        config.providers.model_refusal_ttl(),
    ));
    Ok(ProviderRuntimeOutcome {
        provider: Arc::new(RetryingProvider::new(router, RetryConfig::default())),
        admission_binding_diagnostic: AdmissionBindingDiagnostic {
            unbound_slots: unbound_slots.into_iter().collect(),
        },
    })
}

#[cfg(feature = "test-support")]
fn mock_llm_bare_anthropic_alias_enabled(api_base: &Option<String>) -> bool {
    std::env::var("QUECTO_TAG").ok().as_deref() == Some("mock-llm")
        && api_base.as_deref().is_some_and(|base| {
            base.starts_with("http://127.0.0.1:") || base.starts_with("http://localhost:")
        })
}

#[cfg(test)]
fn registry_provider_factory(
    provider_api: crate::infrastructure::model_registry::ProviderApi,
    provider_prefix: String,
    base: Option<String>,
    allow_remote_http: bool,
    client: reqwest::Client,
) -> crate::infrastructure::providers::refreshable::ProviderFactory {
    registry_provider_factory_with_admission(
        provider_api,
        provider_prefix,
        base,
        allow_remote_http,
        client,
        ProviderBinding::default(),
    )
}

fn registry_provider_factory_with_admission(
    provider_api: crate::infrastructure::model_registry::ProviderApi,
    provider_prefix: String,
    base: Option<String>,
    allow_remote_http: bool,
    client: reqwest::Client,
    binding: ProviderBinding,
) -> crate::infrastructure::providers::refreshable::ProviderFactory {
    use crate::infrastructure::model_registry::ProviderApi;
    Arc::new(move |new_token: &str| -> Arc<dyn LlmProvider> {
        match provider_api {
            ProviderApi::OpenAiCompletions => {
                let base = base
                    .clone()
                    .expect("OpenAI-compatible registry provider base should be validated");
                providers::create_openai_compatible_provider_and_admission(
                    &provider_prefix,
                    new_token.to_string(),
                    base,
                    allow_remote_http,
                    client.clone(),
                    binding.clone(),
                )
                .expect("refreshed OpenAI-compatible registry provider should rebuild")
            }
            ProviderApi::AnthropicMessages => {
                providers::create_anthropic_compatible_provider_and_admission(
                    &provider_prefix,
                    new_token.to_string(),
                    base.clone(),
                    allow_remote_http,
                    client.clone(),
                    binding.clone(),
                )
                .expect("refreshed Anthropic registry provider should rebuild")
            }
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

#[cfg(test)]
fn build_registry_provider(
    model: &crate::infrastructure::model_registry::ModelRecord,
    _base_dir: &std::path::Path,
    store: &Arc<CredentialStore>,
    refresh_fn: &crate::infrastructure::providers::refreshable::RefreshFn,
    http_client: &reqwest::Client,
) -> Result<Option<Arc<dyn LlmProvider>>, String> {
    build_registry_provider_with_admission(model, _base_dir, store, refresh_fn, http_client, None)
}

fn build_registry_provider_with_admission(
    model: &crate::infrastructure::model_registry::ModelRecord,
    _base_dir: &std::path::Path,
    store: &Arc<CredentialStore>,
    refresh_fn: &crate::infrastructure::providers::refreshable::RefreshFn,
    http_client: &reqwest::Client,
    admission: Option<&AdmissionRuntimeContext>,
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
            // #811: use the stored (possibly stale) token; no eager network
            // refresh. RefreshableProvider refreshes lazily on 401 at first use.
            let Some(token) =
                oauth_slot_token(store.get(oauth_provider).map_err(|e| e.to_string())?)
            else {
                return Ok(None);
            };
            api_base = oauth_registry_base_url(model, oauth_provider)?;
            token
        }
    };

    let setting = format!("models.json provider '{}'", model.provider);
    let idle = StreamIdle::configured_for(&setting, model.stream_limits)?;
    let binding = bound(admission, &model.provider, idle)?;
    let inner: Arc<dyn LlmProvider> = match model.api {
        ProviderApi::OpenAiCompletions => {
            let Some(base) = api_base.clone().filter(|b| !b.trim().is_empty()) else {
                return Ok(None);
            };
            providers::create_openai_compatible_provider_and_admission(
                &model.provider,
                auth_key.clone(),
                base,
                model.allow_remote_http,
                http_client.clone(),
                binding.clone(),
            )
            .map_err(|e| format!("models.json provider configuration error: {}", e))?
        }
        ProviderApi::AnthropicMessages => {
            providers::create_anthropic_compatible_provider_and_admission(
                &model.provider,
                auth_key.clone(),
                api_base.clone(),
                model.allow_remote_http,
                http_client.clone(),
                binding.clone(),
            )
            .map_err(|e| format!("models.json provider configuration error: {}", e))?
        }
        ProviderApi::GoogleGenerativeAi => {
            return Err(format!(
                "models.json provider '{}' uses google-generative-ai, but that wire protocol is not implemented yet",
                model.provider
            ));
        }
    };

    if model.auth == AuthMode::OAuth {
        let oauth_provider = model.oauth_provider.clone().expect("validated above");
        let factory = registry_provider_factory_with_admission(
            model.api,
            model.provider.clone(),
            api_base.clone(),
            model.allow_remote_http,
            http_client.clone(),
            binding,
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

/// The binding of the provider in admission slot `slot`, bounded by
/// `stream_idle`, its configured stream idle limit (#2433 review).
fn bound(
    context: Option<&AdmissionRuntimeContext>,
    slot: &str,
    stream_idle: StreamIdle,
) -> Result<ProviderBinding, String> {
    let admission = bound_attempt_transport(context, slot)?;
    Ok(ProviderBinding {
        admission,
        stream_idle,
    })
}

fn bound_attempt_transport(
    context: Option<&AdmissionRuntimeContext>,
    slot: &str,
) -> Result<Option<AttemptTransportBinding>, String> {
    context
        .map(|context| context.optional_binding(slot.trim()))
        .transpose()
        .map(Option::flatten)
}

/// The trimmed value, or `None` when blank. Trimming here keeps the initially
/// composed providers on exactly the base URL the post-refresh factories
/// (which trim via this same helper) rebuild with.
pub(crate) fn non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Build a single provider from name, key, and base URL.
#[cfg(test)]
fn build_single_provider(
    name: &str,
    api_key: &str,
    api_base: &Option<String>,
    http_client: &reqwest::Client,
    disable_codex_routing: bool,
) -> Result<Arc<dyn LlmProvider>, String> {
    build_single_provider_with_admission(
        name,
        api_key,
        api_base,
        http_client,
        disable_codex_routing,
        ProviderBinding::default(),
    )
}

fn build_single_provider_with_admission(
    name: &str,
    api_key: &str,
    api_base: &Option<String>,
    http_client: &reqwest::Client,
    disable_codex_routing: bool,
    binding: ProviderBinding,
) -> Result<Arc<dyn LlmProvider>, String> {
    if name == "openai" && !disable_codex_routing {
        let account_id = crate::infrastructure::auth::oauth::extract_openai_account_id(api_key);
        if let Some(acct) = account_id {
            return providers::create_codex_provider_with_client_and_admission(
                api_key.to_string(),
                acct,
                api_base.clone(),
                http_client.clone(),
                binding.clone(),
            )
            .map_err(|e| format!("openai provider configuration error: {}", e));
        }
    }
    let base = api_base.clone();
    if name == "openai" && disable_codex_routing {
        return providers::create_openai_provider_with_client_and_admission(
            api_key.to_string(),
            base,
            http_client.clone(),
            false,
            binding.clone(),
        )
        .map_err(|e| format!("{} provider configuration error: {}", name, e));
    }
    providers::create_provider_with_client_and_admission(
        name,
        api_key.to_string(),
        base,
        http_client.clone(),
        binding.clone(),
    )
    .map_err(|e| format!("{} provider configuration error: {}", name, e))
}

#[cfg(test)]
#[path = "provider_runtime_cov_tests.rs"]
mod cov_tests;

#[cfg(test)]
#[path = "provider_runtime_stream_idle_tests.rs"]
mod stream_idle_tests;

#[cfg(test)]
#[path = "provider_runtime_xai_tests.rs"]
mod xai_tests;
