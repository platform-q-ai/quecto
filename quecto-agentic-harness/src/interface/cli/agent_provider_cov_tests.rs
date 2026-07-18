use super::*;
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
