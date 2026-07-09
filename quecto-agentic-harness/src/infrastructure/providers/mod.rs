pub mod anthropic;
pub mod codex;
pub mod openai;
pub mod openai_endpoint_router;
pub mod refreshable;
pub mod retry;
pub mod router;
pub mod sse_common;
pub mod usage;

use std::sync::Arc;

use crate::domain::provider::LlmProvider;

const ALLOW_CUSTOM_HOSTS_ENV: &str = "QUECTO_ALLOW_CUSTOM_PROVIDER_HOSTS";
const RESERVED_PROVIDER_PREFIXES: &[&str] =
    &["openai", "openai-codex", "codex", "anthropic", "router"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderFactoryError {
    UnknownProvider(String),
    InvalidApiBase {
        provider: String,
        api_base: String,
        reason: String,
    },
}

impl std::fmt::Display for ProviderFactoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownProvider(name) => write!(f, "unknown provider: {}", name),
            Self::InvalidApiBase {
                provider,
                api_base,
                reason,
            } => write!(
                f,
                "invalid api_base for {}: '{}' ({})",
                provider, api_base, reason
            ),
        }
    }
}

fn is_loopback_host(host: &str) -> bool {
    host == "localhost" || host == "127.0.0.1" || host == "::1"
}

fn allow_custom_provider_hosts() -> bool {
    std::env::var(ALLOW_CUSTOM_HOSTS_ENV)
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn allowed_https_host(provider: &str, host: &str) -> bool {
    if allow_custom_provider_hosts() {
        return true;
    }

    match provider {
        "openai" => host == "api.openai.com",
        "anthropic" => host == "api.anthropic.com",
        _ => false,
    }
}

fn validate_provider_api_base(provider: &str, api_base: &str) -> Result<(), ProviderFactoryError> {
    validate_provider_api_base_with_options(provider, api_base, false, false)
}

fn validate_provider_api_base_with_options(
    provider: &str,
    api_base: &str,
    allow_remote_http: bool,
    allow_any_https_host: bool,
) -> Result<(), ProviderFactoryError> {
    let invalid = |reason: String| ProviderFactoryError::InvalidApiBase {
        provider: provider.to_string(),
        api_base: api_base.to_string(),
        reason,
    };

    let Ok(url) = reqwest::Url::parse(api_base) else {
        return Err(invalid("URL parse failed".to_string()));
    };

    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid("credentials in URL are not allowed".to_string()));
    }

    if url.query().is_some() || url.fragment().is_some() {
        return Err(invalid("query and fragment are not allowed".to_string()));
    }

    let Some(host) = url.host_str() else {
        return Err(invalid("host is missing".to_string()));
    };

    match url.scheme() {
        "https" => {
            if allow_any_https_host || allowed_https_host(provider, host) || is_loopback_host(host)
            {
                Ok(())
            } else {
                Err(invalid(format!(
                    "host '{}' is not allowed (set {}=1 to allow custom hosts)",
                    host, ALLOW_CUSTOM_HOSTS_ENV
                )))
            }
        }
        "http" => {
            if is_loopback_host(host) || allow_remote_http {
                Ok(())
            } else {
                Err(invalid(format!(
                    "http is allowed only for loopback hosts (set {}=1 or allow_remote_http=true on an explicit custom endpoint to allow custom hosts)",
                    ALLOW_CUSTOM_HOSTS_ENV
                )))
            }
        }
        scheme => Err(invalid(format!(
            "unsupported URL scheme '{}'; use https or loopback http",
            scheme
        ))),
    }
}

/// Create a provider by name and API key with a shared `reqwest::Client`.
pub fn create_provider_with_client(
    name: &str,
    api_key: String,
    api_base: Option<String>,
    client: reqwest::Client,
) -> Result<Arc<dyn LlmProvider>, ProviderFactoryError> {
    match name {
        "openai" | "anthropic" => {}
        _ => return Err(ProviderFactoryError::UnknownProvider(name.to_string())),
    }

    if let Some(ref base) = api_base {
        validate_provider_api_base(name, base)?;
    }

    match name {
        "openai" => Ok(Arc::new(openai::OpenAiProvider::with_client(
            api_key, api_base, client,
        ))),
        "anthropic" => Ok(Arc::new(anthropic::AnthropicProvider::with_client(
            api_key, api_base, client,
        ))),
        _ => unreachable!("provider name validated above"),
    }
}

/// Create an OpenAI-compatible/built-in provider with an explicit router prefix.
///
/// #1066: reasoning models registered for `provider_name` are routed per
/// request — reasoning + function tools goes to the Responses API, everything
/// else stays on Chat Completions.
pub fn create_named_openai_provider_with_client(
    provider_name: &str,
    api_key: String,
    api_base: Option<String>,
    client: reqwest::Client,
    include_oauth_headers: bool,
) -> Result<Arc<dyn LlmProvider>, ProviderFactoryError> {
    if let Some(ref base) = api_base {
        validate_provider_api_base("openai", base)?;
    }
    let chat_completions: Arc<dyn LlmProvider> = Arc::new(
        openai::OpenAiProvider::with_client_and_name_and_oauth_headers(
            provider_name,
            api_key.clone(),
            api_base.clone(),
            client.clone(),
            include_oauth_headers,
        ),
    );
    let reasoning_model_ids: std::collections::HashSet<String> =
        crate::infrastructure::model_registry::ModelRegistry::builtin()
            .models()
            .iter()
            .filter(|m| m.provider == provider_name && m.reasoning)
            .map(|m| m.id.clone())
            .collect();
    if reasoning_model_ids.is_empty() {
        return Ok(chat_completions);
    }
    let responses: Arc<dyn LlmProvider> = Arc::new(codex::CodexProvider::with_api_key(
        api_key, api_base, client,
    ));
    Ok(Arc::new(openai_endpoint_router::OpenAiEndpointRouter::new(
        provider_name.to_string(),
        chat_completions,
        responses,
        reasoning_model_ids,
    )))
}

/// Create the built-in OpenAI provider with explicit OAuth-header control.
pub fn create_openai_provider_with_client(
    api_key: String,
    api_base: Option<String>,
    client: reqwest::Client,
    include_oauth_headers: bool,
) -> Result<Arc<dyn LlmProvider>, ProviderFactoryError> {
    if let Some(ref base) = api_base {
        validate_provider_api_base("openai", base)?;
    }
    Ok(Arc::new(
        openai::OpenAiProvider::with_client_and_name_and_oauth_headers(
            "openai",
            api_key,
            api_base,
            client,
            include_oauth_headers,
        ),
    ))
}

/// Create an OpenAI-compatible provider with a custom router prefix.
///
/// Unlike the built-in `openai` slot, this never performs Codex/OAuth routing;
/// it always sends `Authorization: Bearer <api_key>` to the configured base URL.
pub fn create_openai_compatible_provider(
    prefix: &str,
    api_key: String,
    api_base: String,
    allow_remote_http: bool,
    client: reqwest::Client,
) -> Result<Arc<dyn LlmProvider>, ProviderFactoryError> {
    let prefix = prefix.trim();
    if prefix.is_empty()
        || prefix.contains('/')
        || RESERVED_PROVIDER_PREFIXES
            .iter()
            .any(|reserved| prefix.eq_ignore_ascii_case(reserved))
    {
        return Err(ProviderFactoryError::UnknownProvider(prefix.to_string()));
    }
    let allow_remote_http = allow_remote_http || allow_custom_provider_hosts();
    validate_provider_api_base_with_options(prefix, &api_base, allow_remote_http, true)?;
    Ok(Arc::new(
        openai::OpenAiProvider::with_client_and_name_and_oauth_headers(
            prefix,
            api_key,
            Some(api_base),
            client,
            false,
        ),
    ))
}

/// Create an Anthropic-compatible provider with a custom router prefix.
///
/// This uses the kernel-owned Anthropic Messages wire protocol while letting
/// `models.json` expose distinct provider keys such as `anthropic-api` and
/// `anthropic-oauth`.
pub fn create_anthropic_compatible_provider(
    prefix: &str,
    api_key: String,
    api_base: Option<String>,
    allow_remote_http: bool,
    client: reqwest::Client,
) -> Result<Arc<dyn LlmProvider>, ProviderFactoryError> {
    let prefix = prefix.trim();
    if prefix.is_empty()
        || prefix.contains('/')
        || RESERVED_PROVIDER_PREFIXES
            .iter()
            .any(|reserved| prefix.eq_ignore_ascii_case(reserved))
    {
        return Err(ProviderFactoryError::UnknownProvider(prefix.to_string()));
    }
    if let Some(ref base) = api_base {
        let allow_remote_http = allow_remote_http || allow_custom_provider_hosts();
        validate_provider_api_base_with_options(prefix, base, allow_remote_http, true)?;
    }
    Ok(Arc::new(
        anthropic::AnthropicProvider::with_client_and_name(api_key, api_base, client, prefix),
    ))
}

/// Create a Codex provider with a shared `reqwest::Client`.
///
/// Used for ChatGPT OAuth tokens from `auth.openai.com`, which only work
/// against the ChatGPT backend using the Responses API. Requires an
/// `account_id` extracted from the JWT.
pub fn create_codex_provider_with_client(
    api_key: String,
    account_id: String,
    api_base: Option<String>,
    client: reqwest::Client,
) -> Arc<dyn LlmProvider> {
    Arc::new(codex::CodexProvider::with_client(
        api_key, account_id, api_base, client,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_openai_provider() {
        let provider = create_provider_with_client(
            "openai",
            "sk-test".to_string(),
            None,
            reqwest::Client::new(),
        );
        assert!(provider.is_ok());
        assert_eq!(provider.unwrap().name(), "openai");
    }

    #[test]
    fn test_create_anthropic_provider() {
        let provider = create_provider_with_client(
            "anthropic",
            "sk-ant-test".to_string(),
            None,
            reqwest::Client::new(),
        );
        assert!(provider.is_ok());
        assert_eq!(provider.unwrap().name(), "anthropic");
    }

    #[test]
    fn test_create_unknown_provider() {
        let provider =
            create_provider_with_client("gemini", "key".to_string(), None, reqwest::Client::new());
        assert!(matches!(
            provider,
            Err(ProviderFactoryError::UnknownProvider(_))
        ));
    }

    #[test]
    fn test_create_openai_with_custom_base() {
        let provider = create_provider_with_client(
            "openai",
            "sk-test".to_string(),
            Some("http://localhost:8080".to_string()),
            reqwest::Client::new(),
        );
        assert!(provider.is_ok());
    }

    #[test]
    fn test_create_openai_compatible_provider_with_custom_prefix() {
        let provider = create_openai_compatible_provider(
            "spark",
            "sk-spark".to_string(),
            "http://127.0.0.1:8000/v1".to_string(),
            false,
            reqwest::Client::new(),
        );
        assert!(provider.is_ok());
        assert_eq!(provider.unwrap().name(), "spark");
    }

    #[test]
    fn test_create_openai_compatible_rejects_reserved_prefix() {
        let provider = create_openai_compatible_provider(
            "openai",
            "sk-spark".to_string(),
            "http://127.0.0.1:8000/v1".to_string(),
            false,
            reqwest::Client::new(),
        );
        assert!(matches!(
            provider,
            Err(ProviderFactoryError::UnknownProvider(_))
        ));
    }

    #[test]
    fn test_create_openai_compatible_remote_http_requires_opt_in() {
        let rejected = create_openai_compatible_provider(
            "spark",
            "sk-spark".to_string(),
            "http://tailnet-host:8000/v1".to_string(),
            false,
            reqwest::Client::new(),
        );
        assert!(matches!(
            rejected,
            Err(ProviderFactoryError::InvalidApiBase { .. })
        ));

        let allowed = create_openai_compatible_provider(
            "spark",
            "sk-spark".to_string(),
            "http://tailnet-host:8000/v1".to_string(),
            true,
            reqwest::Client::new(),
        );
        assert!(allowed.is_ok());
    }

    #[test]
    fn test_reject_openai_with_insecure_http_api_base() {
        let provider = create_provider_with_client(
            "openai",
            "sk-test".to_string(),
            Some("http://attacker.invalid/v1".to_string()),
            reqwest::Client::new(),
        );
        assert!(matches!(
            provider,
            Err(ProviderFactoryError::InvalidApiBase { .. })
        ));
    }

    #[test]
    fn test_reject_anthropic_with_insecure_http_api_base() {
        let provider = create_provider_with_client(
            "anthropic",
            "sk-ant-test".to_string(),
            Some("http://attacker.invalid".to_string()),
            reqwest::Client::new(),
        );
        assert!(matches!(
            provider,
            Err(ProviderFactoryError::InvalidApiBase { .. })
        ));
    }

    #[test]
    fn test_reject_openai_with_unapproved_https_host() {
        let provider = create_provider_with_client(
            "openai",
            "sk-test".to_string(),
            Some("https://evil.example/v1".to_string()),
            reqwest::Client::new(),
        );
        assert!(matches!(
            provider,
            Err(ProviderFactoryError::InvalidApiBase { .. })
        ));
    }
}
