#[test]
fn runtime_catalogue_marks_builtin_api_key_models_configured_from_effective_api_key_inputs() {
    use crate::infrastructure::config::Config;
    use crate::infrastructure::provider_runtime::build_agent_provider;
    use std::collections::HashMap;

    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = tmp.path().join("missing-config.json");
    let env = HashMap::from([
        ("OPENAI_API_KEY".to_string(), "sk-openai-env".to_string()),
        ("ANTHROPIC_API_KEY".to_string(), "sk-ant-env".to_string()),
    ]);
    let config = Config::load_with_env(config_path.to_str().unwrap(), &env)
        .expect("OPENAI_API_KEY and ANTHROPIC_API_KEY should populate provider config");

    let runtime = build_agent_provider(&config, tmp.path(), &reqwest::Client::new()).unwrap();
    let descriptors = runtime
        .model_descriptors()
        .expect("router should expose descriptors");

    for qualified_id in ["openai-api/gpt-5.6-sol", "anthropic-api/claude-sonnet-5"] {
        let model = descriptors
            .iter()
            .find(|model| model.qualified_id() == qualified_id)
            .unwrap_or_else(|| panic!("{qualified_id} should be advertised"));
        assert!(
            model.configured,
            "{qualified_id} is runnable from the effective API key used to build its provider"
        );
        assert_eq!(
            model.availability,
            crate::domain::catalogue::Availability::Runnable
        );
    }
}

#[test]
fn runtime_catalogue_marks_oauth_model_configured_when_runtime_has_oauth_credential() {
    use crate::infrastructure::auth::credential_store::{AuthMethod, Credential, CredentialStore};
    use crate::infrastructure::config::Config;
    use crate::infrastructure::provider_runtime::build_agent_provider;

    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"openai-oauth":{"api":"openai-completions","baseUrl":"https://api.openai.com/v1","auth":{"mode":"oauth","oauthProvider":"openai"},"models":[{"id":"gpt-oauth"}]}}}"#,
    )
    .unwrap();
    CredentialStore::new(tmp.path())
        .store(Credential {
            provider: "openai".to_string(),
            token: "oauth-token".to_string(),
            method: AuthMethod::OAuth,
            expires_at: None,
            refresh_token: None,
            account_id: None,
        })
        .unwrap();

    let runtime =
        build_agent_provider(&Config::default(), tmp.path(), &reqwest::Client::new()).unwrap();
    let descriptors = runtime
        .model_descriptors()
        .expect("router should expose descriptors");
    let oauth = descriptors
        .iter()
        .find(|model| model.qualified_id() == "openai-oauth/gpt-oauth")
        .expect("oauth model should be advertised");

    assert!(
        oauth.configured,
        "runnable OAuth model must not be advertised as unconfigured"
    );
    assert_eq!(
        oauth.availability,
        crate::domain::catalogue::Availability::Runnable
    );
}

#[test]
fn runtime_catalogue_skips_unsupported_google_oauth_without_rejecting_valid_provider() {
    use crate::domain::catalogue::{Availability, UnavailableReason};
    use crate::infrastructure::config::Config;
    use crate::infrastructure::provider_runtime::build_agent_provider;
    use std::collections::HashMap;

    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{
            "providers": {
                "google-oauth": {
                    "api": "google-generative-ai",
                    "auth": { "mode": "oauth" },
                    "models": [{ "id": "gemini-pro" }]
                },
                "valid-openai": {
                    "api": "openai-completions",
                    "baseUrl": "http://127.0.0.1:9/v1",
                    "auth": { "mode": "apiKey", "apiKey": "sk-valid" },
                    "models": [{ "id": "valid-model" }]
                }
            }
        }"#,
    )
    .unwrap();
    let env = HashMap::new();
    let config = Config::load_with_env(
        tmp.path().join("missing-config.json").to_str().unwrap(),
        &env,
    )
    .expect("valid provider key should load");

    let runtime = build_agent_provider(&config, tmp.path(), &reqwest::Client::new())
        .expect("unsupported Google row must not abort runtime composition");
    let descriptors = runtime
        .model_descriptors()
        .expect("router should expose descriptors");

    assert!(
        descriptors
            .iter()
            .any(|model| model.qualified_id() == "valid-openai/valid-model" && model.configured)
    );
    let google = descriptors
        .iter()
        .find(|model| model.qualified_id() == "google-oauth/gemini-pro")
        .expect("unsupported Google descriptor should be preserved");
    assert!(!google.configured);
    assert!(matches!(
        &google.availability,
        Availability::KnownButUnavailable { reasons }
            if reasons.iter().any(|reason| matches!(
                reason,
                UnavailableReason::InvalidConfiguration(message)
                    if message.contains("no provider was constructed for this prefix")
            ))
    ));
}

#[test]
fn runtime_rejects_openai_compatible_collision_with_unavailable_google_oauth_prefix() {
    use crate::infrastructure::config::{Config, OpenAiCompatibleEndpoint};
    use crate::infrastructure::provider_runtime::build_agent_provider;

    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{
            "providers": {
                "google-oauth": {
                    "api": "google-generative-ai",
                    "auth": { "mode": "oauth" },
                    "models": [{ "id": "gemini-pro" }]
                },
                "valid-openai": {
                    "api": "openai-completions",
                    "baseUrl": "http://127.0.0.1:9/v1",
                    "auth": { "mode": "apiKey", "apiKey": "sk-valid" },
                    "models": [{ "id": "valid-model" }]
                }
            }
        }"#,
    )
    .unwrap();
    let mut config = Config::default();
    config.providers.openai_compatible.endpoints = vec![OpenAiCompatibleEndpoint {
        prefix: "google-oauth".to_string(),
        api_key: "sk-colliding".to_string(),
        api_base: "http://127.0.0.1:10/v1".to_string(),
        allow_remote_http: true,
    }];

    let err = build_agent_provider(&config, tmp.path(), &reqwest::Client::new())
        .expect_err("unavailable catalogue prefixes must not become openai_compatible routes");
    assert!(
        err.contains("duplicate openai_compatible/provider prefix 'google-oauth'"),
        "{err}"
    );
}

#[test]
fn runtime_rejects_openai_compatible_collision_with_every_builtin_prefix() {
    use crate::infrastructure::config::{Config, OpenAiCompatibleEndpoint};
    use crate::infrastructure::provider_runtime::build_agent_provider;
    use crate::infrastructure::providers::RESERVED_PROVIDER_PREFIXES;

    let tmp = tempfile::TempDir::new().unwrap();

    for prefix in RESERVED_PROVIDER_PREFIXES {
        let mut config = Config::default();
        config.providers.openai_compatible.endpoints = vec![OpenAiCompatibleEndpoint {
            prefix: prefix.to_ascii_uppercase(),
            api_key: "sk-colliding".to_string(),
            api_base: "http://127.0.0.1:10/v1".to_string(),
            allow_remote_http: true,
        }];

        let err = build_agent_provider(&config, tmp.path(), &reqwest::Client::new()).expect_err(
            &format!("{prefix} must be reserved before router construction"),
        );
        assert!(
            err.contains("is reserved for a built-in provider"),
            "a reserved prefix is reported as reserved, not as a duplicate the \
             user created — {prefix}: {err}"
        );
    }
}

#[test]
fn openai_compatible_endpoint_completes_a_credential_less_catalogue_route() {
    use crate::infrastructure::config::{Config, OpenAiCompatibleEndpoint};
    use crate::infrastructure::provider_runtime::build_agent_provider;

    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"spark":{"api":"openai-completions","baseUrl":"http://127.0.0.1:9/v1","models":[{"id":"qwen3"}]}}}"#,
    )
    .unwrap();
    let mut config = Config::default();
    config.providers.openai_compatible.endpoints = vec![OpenAiCompatibleEndpoint {
        prefix: "spark".to_string(),
        api_key: "sk-endpoint".to_string(),
        api_base: "http://127.0.0.1:9/v1".to_string(),
        allow_remote_http: true,
    }];

    let runtime = build_agent_provider(&config, tmp.path(), &reqwest::Client::new())
        .expect("an endpoint may supply the credential for a credential-less catalogue route");
    let descriptors = runtime.model_descriptors().expect("router descriptors");
    let spark = descriptors
        .iter()
        .find(|model| model.qualified_id() == "spark/qwen3")
        .expect("the catalogue route stays listed when an endpoint routes it");
    assert_eq!(
        spark.availability,
        crate::domain::catalogue::Availability::Runnable,
        "a model routed through a configured endpoint is runnable"
    );
}

#[test]
fn builtin_prefix_models_are_runnable_when_an_endpoint_supplies_the_key() {
    use crate::infrastructure::config::{Config, OpenAiCompatibleEndpoint};
    use crate::infrastructure::provider_runtime::build_agent_provider;

    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = Config::default();
    config.providers.openai_compatible.endpoints = vec![OpenAiCompatibleEndpoint {
        prefix: "fireworks".to_string(),
        api_key: "sk-endpoint".to_string(),
        api_base: "http://127.0.0.1:9/v1".to_string(),
        allow_remote_http: true,
    }];

    let runtime = build_agent_provider(&config, tmp.path(), &reqwest::Client::new())
        .expect("an endpoint keyed for a builtin prefix must compose");
    let descriptors = runtime.model_descriptors().expect("router descriptors");
    let fireworks: Vec<_> = descriptors
        .iter()
        .filter(|model| model.reference.provider().as_str() == "fireworks")
        .collect();
    assert!(!fireworks.is_empty(), "builtin fireworks models are listed");
    for model in fireworks {
        assert_eq!(
            model.availability,
            crate::domain::catalogue::Availability::Runnable,
            "{} routes through the configured endpoint",
            model.qualified_id()
        );
    }
}

#[test]
fn an_endpoint_pointing_elsewhere_collides_with_a_catalogue_route() {
    use crate::infrastructure::config::{Config, OpenAiCompatibleEndpoint};
    use crate::infrastructure::provider_runtime::build_agent_provider;

    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"spark":{"api":"openai-completions","baseUrl":"http://127.0.0.1:9/v1","auth":{"mode":"apiKey","apiKey":"sk-registry"},"models":[{"id":"qwen3"}]}}}"#,
    )
    .unwrap();
    let mut config = Config::default();
    config.providers.openai_compatible.endpoints = vec![OpenAiCompatibleEndpoint {
        prefix: "spark".to_string(),
        api_key: "sk-endpoint".to_string(),
        api_base: "http://127.0.0.1:10/v1".to_string(),
        allow_remote_http: true,
    }];

    // The endpoint names a different base URL than the catalogue entry, so which
    // one should serve `spark/*` is ambiguous: report it rather than silently
    // redirecting the route.
    let error = build_agent_provider(&config, tmp.path(), &reqwest::Client::new())
        .expect_err("a redirected catalogue route stays ambiguous");
    assert!(
        error.contains("duplicate openai_compatible/provider prefix 'spark'"),
        "{error}"
    );
}

#[test]
fn an_unsupported_transport_keeps_its_reason_when_the_runtime_skips_it() {
    use crate::domain::catalogue::{Availability, TransportKind, UnavailableReason};
    use crate::infrastructure::config::Config;
    use crate::infrastructure::provider_runtime::build_agent_provider;

    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"google":{"api":"google-generative-ai","baseUrl":"https://generativelanguage.example/v1","auth":{"mode":"apiKey","apiKey":"k"},"models":[{"id":"gemini"}]},"open":{"api":"openai-completions","baseUrl":"https://example.test/v1","auth":{"mode":"apiKey","apiKey":"sk-open"},"models":[{"id":"m"}]}}}"#,
    )
    .unwrap();

    let runtime = build_agent_provider(&Config::default(), tmp.path(), &reqwest::Client::new())
        .expect("an unsupported transport must not fail composition");
    let google = runtime
        .model_descriptors()
        .unwrap()
        .iter()
        .find(|model| model.qualified_id() == "google/gemini")
        .expect("the entry stays listed");

    let Availability::KnownButUnavailable { reasons } = &google.availability else {
        panic!("a google entry can never be runnable");
    };
    assert!(
        reasons.contains(&UnavailableReason::UnsupportedTransport {
            transport: TransportKind::GoogleGenerativeAi
        }),
        "the skip must not erase why the transport is unusable: {reasons:?}"
    );
    assert!(
        !google.adapter_supported(),
        "adapter support is derived from that reason"
    );
}

#[test]
fn a_keyless_endpoint_does_not_advertise_its_prefix_as_runnable() {
    use crate::infrastructure::config::{Config, OpenAiCompatibleEndpoint};
    use crate::infrastructure::provider_runtime::build_agent_provider;

    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = Config::default();
    config.providers.openai.api_key = "sk-openai".into();
    config.providers.openai_compatible.endpoints = vec![OpenAiCompatibleEndpoint {
        prefix: "fireworks".to_string(),
        api_key: String::new(),
        api_base: "http://127.0.0.1:9/v1".to_string(),
        allow_remote_http: true,
    }];

    let runtime = build_agent_provider(&config, tmp.path(), &reqwest::Client::new()).unwrap();

    // No provider is constructed for a keyless endpoint, so its prefix must not
    // be advertised as runnable by the catalogue either.
    for model in runtime
        .model_descriptors()
        .unwrap()
        .iter()
        .filter(|model| model.reference.provider().as_str() == "fireworks")
    {
        assert!(
            !model.availability.runnable(),
            "{} has no constructed provider",
            model.qualified_id()
        );
    }
}

#[test]
fn a_catalogue_of_only_unimplemented_transports_reports_why_nothing_composed() {
    use crate::infrastructure::config::Config;
    use crate::infrastructure::provider_runtime::build_agent_provider;

    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"google":{"api":"google-generative-ai","auth":{"mode":"apiKey","apiKey":"k"},"models":[{"id":"gemini"}]}}}"#,
    )
    .unwrap();

    let error = build_agent_provider(&Config::default(), tmp.path(), &reqwest::Client::new())
        .expect_err("a google-only catalogue composes no provider");

    assert!(
        error.contains("google-generative-ai") && error.contains("not implemented"),
        "the user must be told the transport is unimplemented, not to add a key: {error}"
    );
}

#[test]
fn availability_of_an_oauth_record_without_an_oauth_provider_is_false_not_an_error() {
    use crate::infrastructure::config::Config;
    use crate::infrastructure::model_registry::{AuthMode, ModelRecord, ProviderApi};
    use crate::infrastructure::provider_runtime::credentials::{
        CredentialSnapshot, registry_model_credential_available,
    };

    let tmp = tempfile::TempDir::new().unwrap();
    let store = crate::infrastructure::auth::credential_store::CredentialStore::new(tmp.path());
    let credentials = CredentialSnapshot::load(&store).unwrap();
    let record = ModelRecord {
        provider: "anthropic-oauth".to_string(),
        id: "claude".to_string(),
        display_name: None,
        api: ProviderApi::AnthropicMessages,
        base_url: None,
        api_key: None,
        auth_header: true,
        allow_remote_http: false,
        input: Vec::new(),
        context_window: 0,
        max_tokens: 0,
        max_tokens_explicit: false,
        context_window_explicit: false,
        cost: Default::default(),
        reasoning: false,
        auth: AuthMode::OAuth,
        oauth_provider: None,
    };

    // Asking whether a credential exists must not fail a whole composition: an
    // entry that cannot name its OAuth provider simply has none. Constructing
    // that provider still reports the configuration error.
    assert!(
        !registry_model_credential_available(&record, &credentials, &Config::default())
            .expect("an availability query must not error")
    );
}

#[test]
fn an_entry_the_runtime_declines_to_build_is_not_published_as_runnable() {
    use crate::infrastructure::config::Config;
    use crate::infrastructure::provider_runtime::build_agent_provider;

    let tmp = tempfile::TempDir::new().unwrap();
    // A credential but no base URL: `build_registry_provider` skips it, so the
    // catalogue must not advertise it as usable.
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"myllm":{"api":"openai-completions","auth":{"mode":"apiKey","apiKey":"sk-my"},"models":[{"id":"m"}]},"open":{"api":"openai-completions","baseUrl":"https://example.test/v1","auth":{"mode":"apiKey","apiKey":"sk-open"},"models":[{"id":"ok"}]}}}"#,
    )
    .unwrap();

    let runtime = build_agent_provider(&Config::default(), tmp.path(), &reqwest::Client::new())
        .expect("the other provider composes");
    let descriptors = runtime.model_descriptors().unwrap();

    let skipped = descriptors
        .iter()
        .find(|model| model.qualified_id() == "myllm/m")
        .expect("the entry stays listed");
    assert!(
        !skipped.availability.runnable(),
        "an entry with no constructed provider must not be runnable"
    );
    assert!(
        descriptors
            .iter()
            .find(|model| model.qualified_id() == "open/ok")
            .expect("the constructed provider's entry")
            .availability
            .runnable()
    );
}

#[test]
fn an_endpoint_repeating_a_builtin_provider_name_reports_a_duplicate_prefix() {
    use crate::infrastructure::config::{Config, OpenAiCompatibleEndpoint};
    use crate::infrastructure::provider_runtime::build_agent_provider;

    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = Config::default();
    config.providers.openai.api_key = "sk-openai".into();
    config.providers.openai_compatible.endpoints = vec![OpenAiCompatibleEndpoint {
        prefix: "openai-api".to_string(),
        api_key: "sk-endpoint".to_string(),
        api_base: "http://127.0.0.1:9/v1".to_string(),
        allow_remote_http: true,
    }];

    let error = build_agent_provider(&config, tmp.path(), &reqwest::Client::new())
        .expect_err("two definitions of one provider name are ambiguous");

    assert!(
        error.contains("duplicate openai_compatible/provider prefix 'openai-api'"),
        "the collision must be reported as configuration, not as a router invariant: {error}"
    );
}

#[test]
fn a_credential_less_record_does_not_hide_a_later_buildable_one_for_the_same_prefix() {
    use crate::infrastructure::config::Config;
    use crate::infrastructure::provider_runtime::build_agent_provider;

    let tmp = tempfile::TempDir::new().unwrap();
    // A user adding a model under a built-in prefix: the shipped, credential-less
    // records for that prefix are iterated first and must not claim it.
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"fireworks":{"api":"openai-completions","baseUrl":"http://127.0.0.1:9/v1","auth":{"mode":"apiKey","apiKey":"sk-fw"},"models":[{"id":"accounts/fireworks/models/brand-new"}]}}}"#,
    )
    .unwrap();

    let runtime = build_agent_provider(&Config::default(), tmp.path(), &reqwest::Client::new())
        .expect("the keyed record for the prefix must still build");
    let model = runtime
        .model_descriptors()
        .unwrap()
        .iter()
        .find(|model| model.qualified_id() == "fireworks/accounts/fireworks/models/brand-new")
        .expect("the user's model is listed");
    assert!(
        model.availability.runnable(),
        "a keyed record reached construction: {:?}",
        model.availability
    );
}

#[test]
fn shipped_records_are_runnable_once_a_user_key_builds_their_prefix() {
    use crate::infrastructure::config::Config;
    use crate::infrastructure::provider_runtime::build_agent_provider;

    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"fireworks":{"api":"openai-completions","baseUrl":"http://127.0.0.1:9/v1","auth":{"mode":"apiKey","apiKey":"sk-fw"},"models":[{"id":"mine"}]}}}"#,
    )
    .unwrap();

    let runtime = build_agent_provider(&Config::default(), tmp.path(), &reqwest::Client::new())
        .expect("the keyed record builds the prefix");
    let shipped = runtime
        .model_descriptors()
        .unwrap()
        .iter()
        .find(|model| model.qualified_id().starts_with("fireworks/accounts/"))
        .expect("a shipped fireworks model is listed");

    // The provider is constructed per prefix, so the shipped entry routes
    // through it; reporting it as uncredentialled would warn about a model that
    // works.
    assert!(
        shipped.availability.runnable(),
        "{}: {:?}",
        shipped.qualified_id(),
        shipped.availability
    );
    assert!(shipped.configured);
}

#[test]
fn a_case_variant_key_does_not_hide_the_record_that_can_build_the_prefix() {
    use crate::infrastructure::config::Config;
    use crate::infrastructure::provider_runtime::build_agent_provider;

    let tmp = tempfile::TempDir::new().unwrap();
    // "MyCo" sorts before "myco" but carries no credential; the buildable
    // spelling must still construct the route.
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"MyCo":{"api":"openai-completions","baseUrl":"http://127.0.0.1:9/v1","models":[{"id":"a"}]},"myco":{"api":"openai-completions","baseUrl":"http://127.0.0.1:9/v1","auth":{"mode":"apiKey","apiKey":"sk-myco"},"models":[{"id":"b"}]}}}"#,
    )
    .unwrap();

    let runtime = build_agent_provider(&Config::default(), tmp.path(), &reqwest::Client::new())
        .expect("the credentialled spelling builds the prefix");
    let request = crate::domain::provider::ChatRequest {
        messages: &[],
        tools: &[],
        model: "myco/b",
        max_tokens: 16,
        temperature: 0.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let error = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(runtime.chat(request))
        .expect_err("nothing is listening on the configured base");
    assert!(
        error.to_string().contains("127.0.0.1:9"),
        "the route reached its provider: {error}"
    );
}
