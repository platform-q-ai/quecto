use crate::domain::catalogue::Availability;
use crate::infrastructure::config::Config;
use crate::infrastructure::model_registry::{AuthMode, ModelRecord, ProviderApi};
use crate::interface::cli::build_agent_provider;

fn record(provider: &str) -> ModelRecord {
    ModelRecord {
        provider: provider.to_string(),
        id: "m".to_string(),
        display_name: None,
        api: ProviderApi::OpenAiCompletions,
        base_url: None,
        api_key: None,
        auth_header: true,
        allow_remote_http: false,
        input: Vec::new(),
        context_window: 200_000,
        max_tokens: 8_192,
        max_tokens_explicit: false,
        context_window_explicit: false,
        cost: Default::default(),
        reasoning: false,
        auth: AuthMode::ApiKey,
        oauth_provider: None,
    }
}

fn endpoint(
    prefix: &str,
    api_base: &str,
) -> crate::infrastructure::config::OpenAiCompatibleEndpoint {
    crate::infrastructure::config::OpenAiCompatibleEndpoint {
        prefix: prefix.to_string(),
        api_key: "sk-test".to_string(),
        api_base: api_base.to_string(),
        allow_remote_http: true,
    }
}

#[test]
fn skipped_registry_prefix_collides_with_openai_compatible_endpoint() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"local":{"api":"openai-completions","baseUrl":"http://127.0.0.1:9/v1","auth":{"mode":"apiKey"},"models":[{"id":"m"}]}}}"#,
    )
    .unwrap();
    let mut config = Config::default();
    config.providers.openai_compatible.endpoints =
        vec![endpoint("local", "http://127.0.0.1:10/v1")];

    let err = build_agent_provider(&config, tmp.path(), &reqwest::Client::new())
        .expect_err("skipped registry metadata must still reserve its provider prefix");
    assert!(
        err.contains("duplicate openai_compatible/provider prefix"),
        "{err}"
    );
}

#[test]
fn registry_api_key_falls_back_to_builtin_config_and_skips_unknowns() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = crate::infrastructure::auth::credential_store::CredentialStore::new(tmp.path());
    let mut config = Config::default();
    config.providers.anthropic.api_key = "sk-ant-config".to_string();

    assert_eq!(
        super::registry_api_key(&record("anthropic-api"), &store, &config).unwrap(),
        Some("sk-ant-config".to_string())
    );
    assert_eq!(
        super::registry_api_key(&record("custom"), &store, &config).unwrap(),
        None
    );
}

#[test]
fn registry_builtin_provider_uses_config_api_key_when_record_omits_key() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"anthropic-api":{"api":"anthropic-messages","baseUrl":"https://api.anthropic.com","auth":{"mode":"apiKey"},"models":[{"id":"claude-test"}]}}}"#,
    )
    .unwrap();
    let mut config = Config::default();
    config.providers.anthropic.api_key = "sk-ant-config".to_string();

    let runtime = build_agent_provider(&config, tmp.path(), &reqwest::Client::new()).unwrap();
    assert!(runtime.model_descriptors().unwrap().iter().any(|model| {
        model.qualified_id() == "anthropic-api/claude-test" && model.availability.runnable()
    }));
}

#[test]
fn runtime_catalogue_marks_skipped_registry_models_structurally_unavailable() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"ghost":{"api":"openai-completions","baseUrl":"https://api.example.test/v1","auth":{"mode":"apiKey"},"models":[{"id":"m"}]}}}"#,
    )
    .unwrap();
    let mut config = Config::default();
    config.providers.openai.api_key = "sk-test".to_string();

    let runtime = build_agent_provider(&config, tmp.path(), &reqwest::Client::new()).unwrap();
    let descriptors = runtime.model_descriptors().unwrap();
    let ghost = descriptors
        .iter()
        .find(|model| model.qualified_id() == "ghost/m")
        .expect("skipped model should remain represented");
    assert!(matches!(
        ghost.availability,
        Availability::KnownButUnavailable { .. }
    ));
}
