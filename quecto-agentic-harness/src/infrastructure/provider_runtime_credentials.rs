//! Credential and route-ownership rules for composing the provider runtime.
//!
//! Which credential serves a catalogue route, and whether an explicitly
//! configured `openai_compatible` endpoint owns a prefix the catalogue also
//! names, are policy questions separate from wiring the providers together.

use std::collections::{HashMap, HashSet};

use super::CredentialStore;
use crate::infrastructure::auth::credential_store::Credential;
use crate::infrastructure::config::Config;

/// The credential store read once for a whole composition.
///
/// Every registry record used to re-read and re-parse `credentials.json`, which
/// is both slow (once per model, on every startup and reload poll) and
/// inconsistent: a rotation midway through composing would be observed by some
/// records and not others.
pub(crate) struct CredentialSnapshot {
    credentials: HashMap<String, Credential>,
}

impl CredentialSnapshot {
    pub(crate) fn load(store: &CredentialStore) -> Result<Self, String> {
        Ok(Self {
            credentials: store.load_snapshot().map_err(|e| e.to_string())?,
        })
    }

    pub(crate) fn get(&self, provider: &str) -> Option<&Credential> {
        self.credentials.get(provider)
    }
}

pub(super) fn explicit_endpoint_owns_registry_route(
    model: &crate::infrastructure::model_registry::ModelRecord,
    endpoint_prefixes: &HashSet<String>,
    _store: &CredentialSnapshot,
    config: &Config,
) -> Result<bool, String> {
    use crate::infrastructure::model_registry::AuthMode;
    use crate::infrastructure::model_registry::ProviderApi;
    let prefix = model.provider.to_ascii_lowercase();
    // An `openai_compatible` endpoint speaks one wire protocol, so it may only
    // serve a route declared with that protocol: adopting an anthropic-messages
    // or google-generative-ai route would send requests over the wrong wire and
    // fail at turn time. An OAuth route additionally keeps its auth and billing
    // identity while it can actually run: an api-key endpoint must not take over
    // a working OAuth route, though a declared-and-uncredentialled one is not an
    // identity to protect.
    if !endpoint_prefixes.contains(&prefix) || !matches!(model.api, ProviderApi::OpenAiCompletions)
    {
        return Ok(false);
    }
    if matches!(model.auth, AuthMode::OAuth) && oauth_credential_available(model, _store)? {
        return Ok(false);
    }
    let Some(endpoint) = endpoint_for_prefix(&model.provider, config) else {
        return Ok(false);
    };
    let base_agrees = model
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|base| !base.is_empty())
        .is_none_or(|base| same_api_base(base, &endpoint.api_base));
    // The endpoint may serve the prefix only when it does not redirect it: an
    // entry naming a different base URL stays an ambiguous duplicate, whether or
    // not it carries its own credential, and is reported as one.
    Ok(base_agrees)
}

/// `registry_provider_can_construct` ignoring endpoint-supplied credentials, so
/// route ownership does not depend on the endpoint it is being compared with.
/// Whether the record's declared OAuth provider has a usable credential.
fn oauth_credential_available(
    model: &crate::infrastructure::model_registry::ModelRecord,
    store: &CredentialSnapshot,
) -> Result<bool, String> {
    use crate::infrastructure::auth::credential_store::AuthMethod;
    let Some(oauth_provider) = model.oauth_provider.as_deref() else {
        return Ok(false);
    };
    Ok(store
        .get(oauth_provider)
        .is_some_and(|cred| cred.method == AuthMethod::OAuth && !cred.token.is_empty()))
}

fn endpoint_for_prefix<'a>(
    provider: &str,
    config: &'a Config,
) -> Option<&'a crate::infrastructure::config::OpenAiCompatibleEndpoint> {
    let prefix = provider.to_ascii_lowercase();
    config
        .providers
        .openai_compatible
        .endpoints
        .iter()
        .find(|endpoint| {
            !endpoint.api_key.trim().is_empty()
                && endpoint.prefix.trim().to_ascii_lowercase() == prefix
        })
}

fn endpoint_credential_for_prefix(provider: &str, config: &Config) -> bool {
    endpoint_for_prefix(provider, config).is_some()
}

fn same_api_base(left: &str, right: &str) -> bool {
    left.trim()
        .trim_end_matches('/')
        .eq_ignore_ascii_case(right.trim().trim_end_matches('/'))
}

pub(crate) fn registry_model_credential_available(
    model: &crate::infrastructure::model_registry::ModelRecord,
    store: &CredentialSnapshot,
    config: &Config,
) -> Result<bool, String> {
    use crate::infrastructure::auth::credential_store::AuthMethod;
    use crate::infrastructure::model_registry::AuthMode;
    if matches!(
        model.api,
        crate::infrastructure::model_registry::ProviderApi::GoogleGenerativeAi
    ) {
        return Ok(false);
    }
    // A configured endpoint supplies the credential for the prefix it serves,
    // whichever auth path the catalogue entry declares: the endpoint replaces
    // that path rather than satisfying it.
    if endpoint_credential_for_prefix(&model.provider, config) {
        return Ok(true);
    }
    match model.auth {
        AuthMode::ApiKey => Ok(model.api_key.as_deref().is_some_and(|key| !key.is_empty())
            || builtin_api_key_available(model, store, config)?),
        AuthMode::OAuth => {
            // Availability is a question, not a validation: a record that cannot
            // name its OAuth provider simply has no credential. Constructing
            // that provider still reports the configuration error, so a record
            // whose prefix is never built no longer fails the whole composition.
            let Some(oauth_provider) = model.oauth_provider.as_deref() else {
                return Ok(false);
            };
            Ok(store
                .get(oauth_provider)
                .is_some_and(|cred| cred.method == AuthMethod::OAuth && !cred.token.is_empty()))
        }
    }
}
fn builtin_api_key_available(
    model: &crate::infrastructure::model_registry::ModelRecord,
    store: &CredentialSnapshot,
    config: &Config,
) -> Result<bool, String> {
    use crate::infrastructure::auth::credential_store::AuthMethod;
    let (config_key, credential_provider) = match model.provider.as_str() {
        "openai-api" => (&config.providers.openai.api_key, "openai"),
        "anthropic-api" => (&config.providers.anthropic.api_key, "anthropic"),
        _ => return Ok(false),
    };
    if !config_key.is_empty() {
        return Ok(true);
    }
    Ok(store.get(credential_provider).is_some_and(|cred| {
        cred.method == AuthMethod::Token && !cred.token.is_empty() && !cred.is_expired()
    }))
}
#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn registry_api_key(
    model: &crate::infrastructure::model_registry::ModelRecord,
    store: &CredentialSnapshot,
    config: &Config,
) -> Result<Option<String>, String> {
    if let Some(key) = model.api_key.as_ref().filter(|key| !key.is_empty()) {
        return Ok(Some(key.clone()));
    }
    let (config_key, store_name) = match model.provider.as_str() {
        "openai-api" => (config.providers.openai.api_key.as_str(), "openai"),
        "anthropic-api" => (config.providers.anthropic.api_key.as_str(), "anthropic"),
        _ => return Ok(None),
    };
    if !config_key.is_empty() {
        return Ok(Some(config_key.to_string()));
    }
    let Some(cred) = store.get(store_name) else {
        return Ok(None);
    };
    Ok((matches!(
        cred.method,
        crate::infrastructure::auth::credential_store::AuthMethod::Token
    ) && !cred.token.is_empty()
        && !cred.is_expired())
    .then_some(cred.token.clone()))
}
pub(super) fn registry_provider_can_construct(
    model: &crate::infrastructure::model_registry::ModelRecord,
    store: &CredentialSnapshot,
    config: &Config,
) -> Result<bool, String> {
    use crate::infrastructure::model_registry::ProviderApi;
    if matches!(model.api, ProviderApi::GoogleGenerativeAi) {
        return Ok(false);
    }
    // Both auth modes reduce to "is a credential available for this record".
    registry_model_credential_available(model, store, config)
}
