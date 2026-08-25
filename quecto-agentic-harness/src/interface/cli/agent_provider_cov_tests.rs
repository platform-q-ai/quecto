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
    assert!(!err.contains("not a url"), "got: {err}");
}

#[test]
fn validate_oauth_base_url_redacts_credentials_query_and_fragment() {
    let secret = "https://user:token@evil.example/v1?api_key=sk-secret#frag".to_string();
    let err = validate_oauth_base_url(
        "bad-provider",
        "openai",
        Some(&secret),
        "https://api.openai.com/v1",
    )
    .unwrap_err();

    assert!(err.contains("https://evil.example/v1"), "got: {err}");
    for leaked in ["user:token", "token@", "api_key", "sk-secret", "frag"] {
        assert!(!err.contains(leaked), "leaked {leaked}: {err}");
    }
}

#[test]
fn build_registry_provider_skips_api_key_models_without_key() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = Arc::new(CredentialStore::new(tmp.path()));
    let refresh = crate::interface::shared::make_oauth_refresh_fn();
    let mut m = model("custom", ProviderApi::OpenAiCompletions, AuthMode::ApiKey);
    m.base_url = Some("https://api.example.test/v1".to_string());

    let built = build_registry_provider(
        &m,
        tmp.path(),
        &store,
        &refresh,
        &reqwest::Client::new(),
        &Config::default(),
    )
    .expect("provider build should not error");

    assert!(
        built.is_none(),
        "api-key registry model without apiKey must be skipped"
    );
}

#[test]
fn build_registry_provider_skips_unimplemented_google_protocol() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = Arc::new(CredentialStore::new(tmp.path()));
    let refresh = crate::interface::shared::make_oauth_refresh_fn();
    let mut m = model(
        "google-custom",
        ProviderApi::GoogleGenerativeAi,
        AuthMode::ApiKey,
    );
    m.api_key = Some("test-key".to_string());

    let built = build_registry_provider(
        &m,
        tmp.path(),
        &store,
        &refresh,
        &reqwest::Client::new(),
        &Config::default(),
    )
    .unwrap();

    assert!(built.is_none());
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
    let built = build_registry_provider(
        &openai,
        tmp.path(),
        &store,
        &refresh,
        &client,
        &Config::default(),
    )
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
    let built = build_registry_provider(
        &anthropic,
        tmp.path(),
        &store,
        &refresh,
        &client,
        &Config::default(),
    )
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
    let skipped = build_registry_provider(
        &m,
        tmp.path(),
        &store,
        &refresh,
        &reqwest::Client::new(),
        &Config::default(),
    )
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

fn endpoint(
    prefix: &str,
    api_base: &str,
) -> crate::infrastructure::config::OpenAiCompatibleEndpoint {
    crate::infrastructure::config::OpenAiCompatibleEndpoint {
        prefix: prefix.to_string(),
        api_key: "k".to_string(),
        api_base: api_base.to_string(),
        allow_remote_http: true,
    }
}

#[test]
fn build_agent_provider_requires_both_prefix_and_api_base() {
    let tmp = tempfile::TempDir::new().unwrap();
    let client = reqwest::Client::new();

    for (prefix, api_base, case) in [
        ("", "http://127.0.0.1:9/v1", "blank prefix"),
        ("   ", "http://127.0.0.1:9/v1", "whitespace-only prefix"),
        ("p", "", "blank api_base"),
        ("p", "   ", "whitespace-only api_base"),
    ] {
        let mut config = Config::default();
        config.providers.openai_compatible.endpoints = vec![endpoint(prefix, api_base)];

        let err = match build_agent_provider(&config, tmp.path(), &client) {
            Ok(_) => panic!("{case}: incomplete endpoint must be rejected"),
            Err(e) => e,
        };
        assert!(
            err.contains("requires prefix and api_base"),
            "{case}: unexpected error: {err}"
        );
    }
}

#[test]
fn build_agent_provider_rejects_duplicate_endpoint_prefixes_case_insensitively() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = Config::default();
    // Prefixes are canonicalised to lowercase, so these two collide.
    config.providers.openai_compatible.endpoints = vec![
        endpoint("Local", "http://127.0.0.1:9/v1"),
        endpoint("local", "http://127.0.0.1:10/v1"),
    ];

    let err = build_agent_provider(&config, tmp.path(), &reqwest::Client::new())
        .expect_err("a duplicate prefix must be rejected");
    assert!(
        err.contains("duplicate openai_compatible/provider prefix"),
        "expected the duplicate-prefix error, got: {err}"
    );
    // The offending prefix is named in its original casing so the user can find it.
    assert!(
        err.contains("local"),
        "error does not name the prefix: {err}"
    );
}

#[test]
fn build_agent_provider_errors_when_nothing_is_configured() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = Config::default();

    let err = build_agent_provider(&config, tmp.path(), &reqwest::Client::new())
        .expect_err("an empty configuration has no providers to route to");
    assert!(
        err.contains("no LLM providers configured"),
        "expected the no-providers error, got: {err}"
    );
    // The message must tell the user how to fix it.
    assert!(
        err.contains("quecto auth login"),
        "error lacks remediation: {err}"
    );
}

#[test]
fn build_agent_provider_surfaces_a_malformed_models_json() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(tmp.path().join("models.json"), "{ this is not json").unwrap();
    let mut config = Config::default();
    config.providers.openai.api_key = "sk-test".to_string();

    let err = build_agent_provider(&config, tmp.path(), &reqwest::Client::new())
        .expect_err("a malformed models.json must fail the whole build, not be ignored");
    // Must not fall through to "no providers configured": a corrupt override
    // file is a different failure from an absent one.
    assert!(
        !err.contains("no LLM providers configured"),
        "corrupt models.json was silently skipped: {err}"
    );
}

#[test]
fn build_agent_provider_rejects_remote_http_endpoint_unless_opted_in() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = Config::default();
    // Plaintext HTTP to a non-loopback host leaks the API key; it must be
    // refused unless the endpoint explicitly opts in.
    config.providers.openai_compatible.endpoints =
        vec![crate::infrastructure::config::OpenAiCompatibleEndpoint {
            prefix: "remote".to_string(),
            api_key: "k".to_string(),
            api_base: "http://example.invalid/v1".to_string(),
            allow_remote_http: false,
        }];

    let err = build_agent_provider(&config, tmp.path(), &reqwest::Client::new())
        .expect_err("remote plaintext HTTP must be refused without an explicit opt-in");
    assert!(
        err.contains("openai_compatible provider configuration error"),
        "expected the endpoint configuration error, got: {err}"
    );
}

#[test]
fn build_registry_provider_rejects_registry_oauth_models_without_oauth_provider() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = Arc::new(CredentialStore::new(tmp.path()));
    let refresh = crate::interface::shared::make_oauth_refresh_fn();
    let client = reqwest::Client::new();

    let missing_provider = model(
        "missing-oauth-provider",
        ProviderApi::OpenAiCompletions,
        AuthMode::OAuth,
    );
    let err = build_registry_provider(
        &missing_provider,
        tmp.path(),
        &store,
        &refresh,
        &client,
        &Config::default(),
    )
    .expect_err("oauth registry models must name their OAuth provider");
    assert!(err.contains("missing oauthProvider"), "got: {err}");

    let mut unknown_provider = model(
        "unknown-oauth-provider",
        ProviderApi::OpenAiCompletions,
        AuthMode::OAuth,
    );
    unknown_provider.oauth_provider = Some("not-a-provider".to_string());
    let err = build_registry_provider(
        &unknown_provider,
        tmp.path(),
        &store,
        &refresh,
        &client,
        &Config::default(),
    )
    .expect_err("unknown OAuth providers must be rejected");
    assert!(err.contains("not a kernel OAuth provider"), "got: {err}");
}

#[test]
fn build_registry_provider_skips_registry_oauth_models_when_stored_credential_is_not_oauth() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = Arc::new(CredentialStore::new(tmp.path()));
    store
        .store(Credential {
            provider: "openai".to_string(),
            token: "plain-token".to_string(),
            method: AuthMethod::Token,
            expires_at: Some(i64::MAX),
            refresh_token: None,
            account_id: None,
        })
        .unwrap();
    let refresh = crate::interface::shared::make_oauth_refresh_fn();
    let mut token_credential = model(
        "token-credential",
        ProviderApi::OpenAiCompletions,
        AuthMode::OAuth,
    );
    token_credential.oauth_provider = Some("openai".to_string());

    let skipped = build_registry_provider(
        &token_credential,
        tmp.path(),
        &store,
        &refresh,
        &reqwest::Client::new(),
        &Config::default(),
    )
    .expect("token credentials are not OAuth credentials");
    assert!(skipped.is_none());
}

#[test]
fn build_registry_provider_skips_registry_oauth_models_when_stored_oauth_token_is_empty() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = Arc::new(CredentialStore::new(tmp.path()));
    store
        .store(Credential {
            provider: "openai".to_string(),
            token: "".to_string(),
            method: AuthMethod::OAuth,
            expires_at: Some(i64::MAX),
            refresh_token: Some("rt".to_string()),
            account_id: None,
        })
        .unwrap();
    let refresh = crate::interface::shared::make_oauth_refresh_fn();
    let mut token_credential = model(
        "empty-token-credential",
        ProviderApi::OpenAiCompletions,
        AuthMode::OAuth,
    );
    token_credential.oauth_provider = Some("openai".to_string());

    let skipped = build_registry_provider(
        &token_credential,
        tmp.path(),
        &store,
        &refresh,
        &reqwest::Client::new(),
        &Config::default(),
    )
    .expect("empty OAuth credentials are skipped");
    assert!(skipped.is_none());
}

#[test]
fn build_registry_provider_openai_oauth_builds_refreshable_provider() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = Arc::new(CredentialStore::new(tmp.path()));
    store
        .store(Credential {
            provider: "openai".to_string(),
            token: "oauth-token".to_string(),
            method: AuthMethod::OAuth,
            expires_at: Some(i64::MAX),
            refresh_token: Some("rt".to_string()),
            account_id: None,
        })
        .unwrap();
    let refresh = crate::interface::shared::make_oauth_refresh_fn();
    let mut m = model(
        "custom-openai-oauth",
        ProviderApi::OpenAiCompletions,
        AuthMode::OAuth,
    );
    m.oauth_provider = Some("openai".to_string());
    m.base_url = Some("https://api.openai.com/v1".to_string());

    let built = build_registry_provider(
        &m,
        tmp.path(),
        &store,
        &refresh,
        &reqwest::Client::new(),
        &Config::default(),
    )
    .unwrap()
    .expect("valid OAuth registry provider");
    assert_eq!(built.name(), "custom-openai-oauth");
    assert!(
        built
            .as_any()
            .downcast_ref::<RefreshableProvider>()
            .is_some(),
        "OAuth registry providers must be wrapped for lazy token refresh"
    );
}

#[test]
fn build_registry_provider_xai_oauth_builds_openai_compatible_refreshable_provider() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = Arc::new(CredentialStore::new(tmp.path()));
    store
        .store(Credential {
            provider: "xai".to_string(),
            token: "xai-oauth-token".to_string(),
            method: AuthMethod::OAuth,
            expires_at: Some(i64::MAX),
            refresh_token: Some("rt".to_string()),
            account_id: None,
        })
        .unwrap();
    let refresh = crate::interface::shared::make_oauth_refresh_fn();
    let mut m = model(
        "xai-custom",
        ProviderApi::OpenAiCompletions,
        AuthMode::OAuth,
    );
    m.oauth_provider = Some("xai".to_string());
    m.base_url = Some("https://api.x.ai/v1".to_string());

    let built = build_registry_provider(
        &m,
        tmp.path(),
        &store,
        &refresh,
        &reqwest::Client::new(),
        &Config::default(),
    )
    .unwrap()
    .expect("valid xAI OAuth registry provider");
    assert_eq!(built.name(), "xai-custom");
    assert!(
        built
            .as_any()
            .downcast_ref::<RefreshableProvider>()
            .is_some(),
        "xAI OAuth registry providers must be wrapped for lazy token refresh"
    );
}

#[test]
fn build_registry_provider_rejects_noncanonical_openai_oauth_base_url() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = Arc::new(CredentialStore::new(tmp.path()));
    store
        .store(Credential {
            provider: "openai".to_string(),
            token: "oauth-token".to_string(),
            method: AuthMethod::OAuth,
            expires_at: Some(i64::MAX),
            refresh_token: Some("rt".to_string()),
            account_id: None,
        })
        .unwrap();
    let refresh = crate::interface::shared::make_oauth_refresh_fn();
    let mut m = model(
        "evil-openai-oauth",
        ProviderApi::OpenAiCompletions,
        AuthMode::OAuth,
    );
    m.oauth_provider = Some("openai".to_string());
    m.base_url = Some("https://attacker.example/v1".to_string());

    let err = build_registry_provider(
        &m,
        tmp.path(),
        &store,
        &refresh,
        &reqwest::Client::new(),
        &Config::default(),
    )
    .expect_err("OAuth registry providers must pin canonical hosts");
    assert!(err.contains("canonical OAuth host"), "got: {err}");
    assert!(err.contains("evil-openai-oauth"), "got: {err}");
}

#[test]
fn build_single_provider_reports_configuration_errors_for_a_bad_api_base() {
    // build_single_provider's error arm: an api_base that fails validation is
    // reported against the provider name rather than panicking or silently
    // yielding a provider that would fail later at request time.
    let err = build_single_provider(
        "openai",
        "sk-test",
        &Some("http://example.invalid/v1".to_string()),
        &reqwest::Client::new(),
        true,
    )
    .expect_err("an invalid api_base must be reported at construction time");
    assert!(
        err.contains("openai") && err.contains("provider configuration error"),
        "error should name the provider and the stage: {err}"
    );
}
