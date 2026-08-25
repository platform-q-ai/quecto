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
                    if message.contains("provider skipped during runtime construction")
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
            err.contains("duplicate openai_compatible/provider prefix"),
            "{prefix}: {err}"
        );
    }
}
