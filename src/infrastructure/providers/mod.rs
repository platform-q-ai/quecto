pub mod anthropic;
pub mod codex;
pub mod error;
pub mod fallback;
pub mod openai;

use std::sync::Arc;

use crate::domain::provider::LlmProvider;

const ALLOW_CUSTOM_HOSTS_ENV: &str = "QUECTO_ALLOW_CUSTOM_PROVIDER_HOSTS";

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
    let Ok(url) = reqwest::Url::parse(api_base) else {
        return Err(ProviderFactoryError::InvalidApiBase {
            provider: provider.to_string(),
            api_base: api_base.to_string(),
            reason: "URL parse failed".to_string(),
        });
    };

    if !url.username().is_empty() || url.password().is_some() {
        return Err(ProviderFactoryError::InvalidApiBase {
            provider: provider.to_string(),
            api_base: api_base.to_string(),
            reason: "credentials in URL are not allowed".to_string(),
        });
    }

    if url.query().is_some() || url.fragment().is_some() {
        return Err(ProviderFactoryError::InvalidApiBase {
            provider: provider.to_string(),
            api_base: api_base.to_string(),
            reason: "query and fragment are not allowed".to_string(),
        });
    }

    let Some(host) = url.host_str() else {
        return Err(ProviderFactoryError::InvalidApiBase {
            provider: provider.to_string(),
            api_base: api_base.to_string(),
            reason: "host is missing".to_string(),
        });
    };

    match url.scheme() {
        "https" => {
            if allowed_https_host(provider, host) || is_loopback_host(host) {
                Ok(())
            } else {
                Err(ProviderFactoryError::InvalidApiBase {
                    provider: provider.to_string(),
                    api_base: api_base.to_string(),
                    reason: format!(
                        "host '{}' is not allowed (set {}=1 to allow custom hosts)",
                        host, ALLOW_CUSTOM_HOSTS_ENV
                    ),
                })
            }
        }
        "http" => {
            if is_loopback_host(host) {
                Ok(())
            } else {
                Err(ProviderFactoryError::InvalidApiBase {
                    provider: provider.to_string(),
                    api_base: api_base.to_string(),
                    reason: "http is allowed only for loopback hosts".to_string(),
                })
            }
        }
        scheme => Err(ProviderFactoryError::InvalidApiBase {
            provider: provider.to_string(),
            api_base: api_base.to_string(),
            reason: format!(
                "unsupported URL scheme '{}'; use https or loopback http",
                scheme
            ),
        }),
    }
}

/// Create a provider by name and API key.
pub fn create_provider(
    name: &str,
    api_key: String,
    api_base: Option<String>,
) -> Result<Arc<dyn LlmProvider>, ProviderFactoryError> {
    create_provider_with_client(name, api_key, api_base, reqwest::Client::new())
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

/// Create a Codex provider for ChatGPT OAuth tokens.
///
/// OAuth tokens from `auth.openai.com` only work against the ChatGPT
/// backend using the Responses API. Requires an `account_id` extracted
/// from the JWT.
pub fn create_codex_provider(api_key: String, account_id: String) -> Arc<dyn LlmProvider> {
    create_codex_provider_with_client(api_key, account_id, reqwest::Client::new())
}

/// Create a Codex provider with a shared `reqwest::Client`.
pub fn create_codex_provider_with_client(
    api_key: String,
    account_id: String,
    client: reqwest::Client,
) -> Arc<dyn LlmProvider> {
    Arc::new(codex::CodexProvider::with_client(
        api_key, account_id, None, client,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_openai_provider() {
        let provider = create_provider("openai", "sk-test".to_string(), None);
        assert!(provider.is_ok());
        assert_eq!(provider.unwrap().name(), "openai");
    }

    #[test]
    fn test_create_anthropic_provider() {
        let provider = create_provider("anthropic", "sk-ant-test".to_string(), None);
        assert!(provider.is_ok());
        assert_eq!(provider.unwrap().name(), "anthropic");
    }

    #[test]
    fn test_create_unknown_provider() {
        let provider = create_provider("gemini", "key".to_string(), None);
        assert!(matches!(
            provider,
            Err(ProviderFactoryError::UnknownProvider(_))
        ));
    }

    #[test]
    fn test_create_openai_with_custom_base() {
        let provider = create_provider(
            "openai",
            "sk-test".to_string(),
            Some("http://localhost:8080".to_string()),
        );
        assert!(provider.is_ok());
    }

    #[test]
    fn test_reject_openai_with_insecure_http_api_base() {
        let provider = create_provider(
            "openai",
            "sk-test".to_string(),
            Some("http://attacker.invalid/v1".to_string()),
        );
        assert!(matches!(
            provider,
            Err(ProviderFactoryError::InvalidApiBase { .. })
        ));
    }

    #[test]
    fn test_reject_anthropic_with_insecure_http_api_base() {
        let provider = create_provider(
            "anthropic",
            "sk-ant-test".to_string(),
            Some("http://attacker.invalid".to_string()),
        );
        assert!(matches!(
            provider,
            Err(ProviderFactoryError::InvalidApiBase { .. })
        ));
    }

    #[test]
    fn test_reject_openai_with_unapproved_https_host() {
        let provider = create_provider(
            "openai",
            "sk-test".to_string(),
            Some("https://evil.example/v1".to_string()),
        );
        assert!(matches!(
            provider,
            Err(ProviderFactoryError::InvalidApiBase { .. })
        ));
    }
}
