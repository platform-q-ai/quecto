use super::*;
use crate::domain::catalogue::{AuthIdentity, Availability, TransportKind, UnavailableReason};
use crate::infrastructure::model_registry::ModelRegistry;

#[test]
fn registry_source_translates_legacy_models_to_secret_free_domain_descriptors() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"openai-api":{"api":"openai-completions","baseUrl":"https://api.openai.com/v1","auth":{"mode":"apiKey","apiKey":"sk-secret"},"models":[{"id":"gpt-custom","name":"Custom GPT","contextWindow":1000,"maxTokens":200,"input":["text","image"],"reasoning":true,"cost":{"input":1.0,"output":2.0,"cacheRead":0.1,"cacheWrite":0.2}}]}}}"#,
    )
    .unwrap();
    let source =
        ModelRegistryCatalogueSource::load_from_path(&tmp.path().join("models.json")).unwrap();

    let models = source.load().unwrap();

    let model = models
        .iter()
        .find(|model| model.qualified_id() == "openai-api/gpt-custom")
        .expect("custom model should be present in the effective registry projection");
    assert_eq!(model.reference.provider().as_str(), "openai-api");
    assert_eq!(model.reference.model().as_str(), "gpt-custom");
    assert_eq!(model.qualified_id(), "openai-api/gpt-custom");
    assert_eq!(model.transport, TransportKind::OpenAiCompletions);
    assert_eq!(model.auth, AuthIdentity::ApiKey);
    assert_eq!(model.base_url.as_deref(), Some("https://api.openai.com/v1"));
    assert!(model.configured);
    assert_eq!(model.capabilities.context_window, 1000);
    assert!(model.capabilities.context_window_explicit);
    assert_eq!(model.capabilities.max_tokens, 200);
    assert!(model.capabilities.max_tokens_explicit);
    assert_eq!(
        model.capabilities.input,
        ["text".to_string(), "image".to_string()]
    );
    assert!(model.capabilities.reasoning);
    assert_eq!(model.capabilities.cost.cache_write, 0.2);
    assert_eq!(model.availability, Availability::Runnable);
}

#[test]
fn registry_source_skips_records_with_blank_provider_or_model_ids() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"openish":{"api":"openai-completions","models":[{"id":"valid"},{"id":"   "}]},"   ":{"api":"openai-completions","models":[{"id":"blank-provider"}]}}}"#,
    )
    .unwrap();
    let source =
        ModelRegistryCatalogueSource::load_from_path(&tmp.path().join("models.json")).unwrap();

    let models = source.load().unwrap();

    assert!(
        models
            .iter()
            .any(|model| model.qualified_id() == "openish/valid")
    );
    assert!(
        !models
            .iter()
            .any(|model| model.qualified_id() == "   /blank-provider")
    );
    assert!(
        !models
            .iter()
            .any(|model| model.qualified_id() == "openish/   ")
    );
}

#[test]
fn registry_source_keeps_auth_identity_separate_and_marks_missing_runtime_capability_structurally()
{
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"anthropic-oauth":{"api":"anthropic-messages","auth":{"mode":"oauth","oauthProvider":"anthropic"},"models":[{"id":"claude"}]},"google":{"api":"google-generative-ai","auth":{"mode":"apiKey"},"models":[{"id":"gemini"}]}}}"#,
    )
    .unwrap();
    let registry = ModelRegistry::load_from_path(&tmp.path().join("models.json")).unwrap();
    let source = ModelRegistryCatalogueSource::new(registry);
    assert_eq!(source.name(), "models-registry");

    let models = source.load().unwrap();

    let anthropic = models
        .iter()
        .find(|m| m.qualified_id() == "anthropic-oauth/claude")
        .unwrap();
    assert_eq!(
        anthropic.auth,
        AuthIdentity::OAuth {
            provider: crate::domain::catalogue::ProviderId::new("anthropic").unwrap()
        }
    );
    assert_eq!(
        anthropic.availability,
        Availability::KnownButUnavailable {
            reasons: vec![UnavailableReason::MissingCredential]
        }
    );

    let google = models
        .iter()
        .find(|m| m.qualified_id() == "google/gemini")
        .unwrap();
    assert_eq!(google.transport, TransportKind::GoogleGenerativeAi);
    assert_eq!(
        google.availability,
        Availability::KnownButUnavailable {
            reasons: vec![
                UnavailableReason::UnsupportedTransport {
                    transport: TransportKind::GoogleGenerativeAi
                },
                UnavailableReason::MissingCredential,
            ]
        }
    );

    let custom_oauth_record = ModelRecord {
        provider: "custom-oauth".to_string(),
        id: "model".to_string(),
        display_name: None,
        api: ProviderApi::OpenAiCompletions,
        base_url: None,
        api_key: None,
        auth_header: true,
        allow_remote_http: false,
        input: Vec::new(),
        context_window: 0,
        max_tokens: 0,
        max_tokens_explicit: false,
        context_window_explicit: false,
        cost: crate::infrastructure::model_registry::ModelCost::default(),
        reasoning: false,
        auth: AuthMode::OAuth,
        oauth_provider: Some("   ".to_string()),
    };
    let error = record_to_descriptor(&custom_oauth_record).unwrap_err();
    assert!(error.contains("provider id must not be empty"));
}
