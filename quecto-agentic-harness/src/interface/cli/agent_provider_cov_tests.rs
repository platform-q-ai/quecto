use super::*;
use crate::infrastructure::auth::credential_store::{AuthMethod, Credential};
use crate::infrastructure::model_registry::{AuthMode, ModelCost, ModelRecord, ProviderApi};

fn model(provider: &str, api: ProviderApi, auth: AuthMode) -> ModelRecord {
    ModelRecord {
        provider: provider.to_string(),
        id: "model-1".to_string(),
        display_name: None,
        api,
        base_url: None,
        api_key: None,
        auth_header: true,
        allow_remote_http: false,
        input: vec!["text".to_string()],
        context_window: 128_000,
        max_tokens: 4096,
        max_tokens_explicit: false,
        context_window_explicit: false,
        cost: ModelCost::default(),
        reasoning: false,
        auth,
        oauth_provider: None,
    }
}

#[test]
fn non_empty_trims_blank_strings() {
    assert_eq!(
        non_empty(" https://example.test ".to_string()).as_deref(),
        Some(" https://example.test ")
    );
    assert_eq!(non_empty("   \t\n".to_string()), None);
}

#[test]
fn openai_oauth_base_url_defaults_to_canonical() {
    let m = model(
        "custom-openai",
        ProviderApi::OpenAiCompletions,
        AuthMode::OAuth,
    );
    let got = oauth_registry_base_url(&m, "openai").unwrap();
    assert_eq!(got.as_deref(), Some("https://api.openai.com/v1"));
}

#[test]
fn anthropic_oauth_base_url_defaults_to_canonical() {
    let m = model(
        "custom-anthropic",
        ProviderApi::AnthropicMessages,
        AuthMode::OAuth,
    );
    let got = oauth_registry_base_url(&m, "anthropic").unwrap();
    assert_eq!(got.as_deref(), Some("https://api.anthropic.com"));
}

#[test]
fn google_oauth_base_url_preserves_configured_without_host_pinning() {
    let mut m = model("google", ProviderApi::GoogleGenerativeAi, AuthMode::OAuth);
    m.base_url = Some("https://generativelanguage.googleapis.com/v1beta".to_string());
    let got = oauth_registry_base_url(&m, "google").unwrap();
    assert_eq!(
        got.as_deref(),
        Some("https://generativelanguage.googleapis.com/v1beta")
    );
}

#[test]
fn oauth_base_url_rejects_incompatible_api_and_oauth_provider() {
    let m = model("bad", ProviderApi::AnthropicMessages, AuthMode::OAuth);
    let err = oauth_registry_base_url(&m, "openai").unwrap_err();
    assert!(err.contains("incompatible api"), "got: {err}");
    assert!(err.contains("bad"), "got: {err}");
}

#[test]
fn validate_oauth_base_url_rejects_malformed_configured_url() {
    let err = validate_oauth_base_url(
        "bad-provider",
        "openai",
        Some(&"not a url".to_string()),
        "https://api.openai.com/v1",
    )
    .unwrap_err();
    assert!(err.contains("invalid OAuth baseUrl"), "got: {err}");
    assert!(err.contains("bad-provider"), "got: {err}");
}

#[test]
fn build_registry_provider_skips_api_key_models_without_key() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = Arc::new(CredentialStore::new(tmp.path()));
    let refresh = crate::interface::shared::make_oauth_refresh_fn();
    let mut m = model("custom", ProviderApi::OpenAiCompletions, AuthMode::ApiKey);
    m.base_url = Some("https://api.example.test/v1".to_string());

    let built = build_registry_provider(&m, tmp.path(), &store, &refresh, &reqwest::Client::new())
        .expect("provider build should not error");

    assert!(
        built.is_none(),
        "api-key registry model without apiKey must be skipped"
    );
}

#[test]
fn build_registry_provider_reports_unimplemented_google_protocol() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = Arc::new(CredentialStore::new(tmp.path()));
    let refresh = crate::interface::shared::make_oauth_refresh_fn();
    let mut m = model(
        "google-custom",
        ProviderApi::GoogleGenerativeAi,
        AuthMode::ApiKey,
    );
    m.api_key = Some("test-key".to_string());

    let err = build_registry_provider(&m, tmp.path(), &store, &refresh, &reqwest::Client::new())
        .unwrap_err();

    assert!(err.contains("google-generative-ai"), "got: {err}");
    assert!(err.contains("not implemented"), "got: {err}");
}

#[test]
fn registry_provider_factory_builds_openai_and_anthropic_refresh_targets() {
    let client = reqwest::Client::new();
    let openai_factory = registry_provider_factory(
        ProviderApi::OpenAiCompletions,
        "custom-openai".to_string(),
        Some("http://127.0.0.1:9/v1".to_string()),
        true,
        client.clone(),
    );
    let openai = openai_factory("fresh-openai-token");
    assert_eq!(openai.name(), "custom-openai");

    let anthropic_factory = registry_provider_factory(
        ProviderApi::AnthropicMessages,
        "custom-anthropic".to_string(),
        Some("http://127.0.0.1:9".to_string()),
        true,
        client,
    );
    let anthropic = anthropic_factory("fresh-anthropic-token");
    assert_eq!(anthropic.name(), "custom-anthropic");
}

#[test]
fn build_registry_provider_api_key_openai_and_oauth_anthropic_paths() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = Arc::new(CredentialStore::new(tmp.path()));
    store
        .store(Credential {
            provider: "anthropic".to_string(),
            token: "oauth-token".to_string(),
            method: AuthMethod::OAuth,
            expires_at: Some(i64::MAX),
            refresh_token: Some("rt".to_string()),
            account_id: None,
        })
        .unwrap();
    let refresh = crate::interface::shared::make_oauth_refresh_fn();
    let client = reqwest::Client::new();

    let mut openai = model(
        "custom-openai",
        ProviderApi::OpenAiCompletions,
        AuthMode::ApiKey,
    );
    openai.base_url = Some("http://127.0.0.1:9/v1".to_string());
    openai.allow_remote_http = true;
    openai.api_key = Some("api-token".to_string());
    let built = build_registry_provider(&openai, tmp.path(), &store, &refresh, &client)
        .unwrap()
        .expect("api-key provider");
    assert_eq!(built.name(), "custom-openai");

    let mut anthropic = model(
        "custom-anthropic-oauth",
        ProviderApi::AnthropicMessages,
        AuthMode::OAuth,
    );
    anthropic.oauth_provider = Some("anthropic".to_string());
    anthropic.base_url = Some("https://api.anthropic.com".to_string());
    let built = build_registry_provider(&anthropic, tmp.path(), &store, &refresh, &client)
        .unwrap()
        .expect("oauth provider");
    assert_eq!(built.name(), "custom-anthropic-oauth");
}

#[test]
fn build_agent_provider_wraps_configured_builtin_providers() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = Config::default();
    config.providers.openai.api_key = "sk-test".to_string();
    config.providers.openai.api_base = "http://127.0.0.1:9/v1".to_string();
    config.providers.anthropic.api_key = "sk-ant-test".to_string();
    config.providers.anthropic.api_base = "http://127.0.0.1:9".to_string();

    let built = build_agent_provider(&config, tmp.path(), &reqwest::Client::new()).unwrap();
    assert_eq!(built.name(), "router");
}

#[test]
fn validate_oauth_base_url_accepts_canonical_host_with_path_and_registry_skips_oauth_without_credentials()
 {
    let configured = "https://api.openai.com/v1/responses".to_string();
    let accepted = validate_oauth_base_url(
        "custom-openai",
        "openai",
        Some(&configured),
        "https://api.openai.com/v1",
    )
    .unwrap();
    assert_eq!(accepted, configured);

    let rejected = validate_oauth_base_url(
        "custom-openai",
        "openai",
        Some(&"https://evil.example/v1".to_string()),
        "https://api.openai.com/v1",
    )
    .unwrap_err();
    assert!(rejected.contains("canonical OAuth host"), "{rejected}");

    let tmp = tempfile::TempDir::new().unwrap();
    let store = Arc::new(CredentialStore::new(tmp.path()));
    let refresh = crate::interface::shared::make_oauth_refresh_fn();
    let mut m = model(
        "custom-openai-oauth",
        ProviderApi::OpenAiCompletions,
        AuthMode::OAuth,
    );
    m.oauth_provider = Some("openai".to_string());
    m.base_url = Some("https://api.openai.com/v1".to_string());
    let skipped =
        build_registry_provider(&m, tmp.path(), &store, &refresh, &reqwest::Client::new())
            .expect("missing credential is a skip");
    assert!(skipped.is_none());
}

#[test]
fn build_agent_provider_rejects_endpoint_over_limit_before_provider_creation() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = Config::default();
    let template = crate::infrastructure::config::OpenAiCompatibleEndpoint {
        prefix: "p".to_string(),
        api_key: "k".to_string(),
        api_base: "http://127.0.0.1:9/v1".to_string(),
        allow_remote_http: true,
    };
    config.providers.openai_compatible.endpoints = (0..33)
        .map(
            |idx| crate::infrastructure::config::OpenAiCompatibleEndpoint {
                prefix: format!("p{idx}"),
                ..template.clone()
            },
        )
        .collect();

    let err = build_agent_provider(&config, tmp.path(), &reqwest::Client::new()).unwrap_err();
    assert!(err.contains("exceeding the maximum"), "{err}");
}
