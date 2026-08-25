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
