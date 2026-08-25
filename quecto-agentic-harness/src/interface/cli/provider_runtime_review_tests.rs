use crate::domain::catalogue::Availability;
use crate::domain::catalogue::ModelDescriptor;
use crate::domain::provider::LlmProvider;
use crate::infrastructure::auth::credential_store::{AuthMethod, Credential, CredentialStore};
use crate::infrastructure::config::{Config, OpenAiCompatibleEndpoint};
use crate::infrastructure::provider_runtime::build_agent_provider;
use crate::infrastructure::providers::retry::RetryingProvider;
use crate::infrastructure::providers::router::ProviderRouter;

fn oauth_credential(provider: &str, token: &str) -> Credential {
    Credential {
        provider: provider.to_string(),
        token: token.to_string(),
        method: AuthMethod::OAuth,
        expires_at: None,
        refresh_token: Some("refresh".to_string()),
        account_id: None,
    }
}

fn router(provider: &dyn LlmProvider) -> &ProviderRouter {
    provider
        .as_any()
        .downcast_ref::<RetryingProvider>()
        .and_then(|p| p.inner().as_any().downcast_ref::<ProviderRouter>())
        .unwrap()
}

#[test]
fn builtin_openai_oauth_rejects_non_canonical_configured_base_without_leaking_token() {
    let tmp = tempfile::tempdir().unwrap();
    CredentialStore::new(tmp.path())
        .store(oauth_credential("openai", "SECRET_OPENAI_BEARER"))
        .unwrap();
    let mut config = Config::default();
    config.providers.openai.api_base =
        "https://evil.example/v1?token=SECRET_OPENAI_BEARER".to_string();

    let err = build_agent_provider(&config, tmp.path(), &reqwest::Client::new()).unwrap_err();

    assert!(err.contains("openai-oauth"));
    assert!(err.contains("canonical OAuth host"));
    assert!(!err.contains("SECRET_OPENAI_BEARER"));
}

#[test]
fn builtin_anthropic_oauth_rejects_non_canonical_configured_base_without_leaking_token() {
    let tmp = tempfile::tempdir().unwrap();
    CredentialStore::new(tmp.path())
        .store(oauth_credential("anthropic", "SECRET_ANTHROPIC_BEARER"))
        .unwrap();
    let mut config = Config::default();
    config.providers.anthropic.api_base =
        "https://evil.example?token=SECRET_ANTHROPIC_BEARER".to_string();

    let err = build_agent_provider(&config, tmp.path(), &reqwest::Client::new()).unwrap_err();

    assert!(err.contains("anthropic-oauth"));
    assert!(err.contains("canonical OAuth host"));
    assert!(!err.contains("SECRET_ANTHROPIC_BEARER"));
}

#[test]
fn unsupported_google_registry_rows_are_structured_unavailable_not_fatal() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"openai-api":{"api":"openai-completions","baseUrl":"https://api.openai.com/v1","auth":{"mode":"apiKey","env":"OPENAI_API_KEY"},"models":[{"id":"gpt-4o"}]},"google":{"api":"google-generative-ai","auth":{"mode":"apiKey","env":"GOOGLE_API_KEY"},"models":[{"id":"gemini"}]}}}"#,
    )
    .unwrap();
    let mut config = Config::default();
    config.providers.openai.api_key = "sk-test".to_string();

    let provider = build_agent_provider(&config, tmp.path(), &reqwest::Client::new()).unwrap();
    let descriptors =
        crate::infrastructure::catalogue_registry::ModelRegistryCatalogueSource::load_from_path(
            &tmp.path().join("models.json"),
        )
        .unwrap()
        .load_valid_descriptors()
        .unwrap();
    let google = descriptors
        .iter()
        .find(|d| d.qualified_id() == "google/gemini")
        .unwrap();

    assert!(matches!(
        google.availability,
        Availability::KnownButUnavailable { .. }
    ));
    assert!(
        router(provider.as_ref())
            .provider_names()
            .contains(&"openai-api")
    );
}

#[test]
fn oauth_only_registry_prefix_blocks_openai_compatible_collision() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"dupe":{"api":"openai-completions","auth":{"mode":"oauth","oauthProvider":"openai"},"models":[{"id":"gpt"}]}}}"#,
    )
    .unwrap();
    let mut config = Config::default();
    config.providers.openai.api_key = "sk-test".to_string();
    config
        .providers
        .openai_compatible
        .endpoints
        .push(OpenAiCompatibleEndpoint {
            prefix: "dupe".to_string(),
            api_key: "sk-compatible".to_string(),
            api_base: "https://compatible.example/v1".to_string(),
            allow_remote_http: false,
        });

    let err = build_agent_provider(&config, tmp.path(), &reqwest::Client::new()).unwrap_err();

    assert!(err.contains("duplicate openai_compatible/provider prefix 'dupe'"));
}

#[test]
fn real_runtime_descriptors_mark_configured_from_actual_config_credentials() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"openai-api":{"api":"openai-completions","baseUrl":"https://api.openai.com/v1","auth":{"mode":"apiKey","env":"OPENAI_API_KEY"},"models":[{"id":"gpt-4o"}]}}}"#,
    )
    .unwrap();
    let mut config = Config::default();
    config.providers.openai.api_key = "sk-test".to_string();
    build_agent_provider(&config, tmp.path(), &reqwest::Client::new()).unwrap();
    let descriptors =
        crate::infrastructure::catalogue_registry::ModelRegistryCatalogueSource::load_from_path(
            &tmp.path().join("models.json"),
        )
        .unwrap()
        .load_valid_descriptors()
        .unwrap();
    let custom: &ModelDescriptor = descriptors
        .iter()
        .find(|d| d.qualified_id() == "openai-api/gpt-4o")
        .unwrap();

    assert!(custom.configured);
}
