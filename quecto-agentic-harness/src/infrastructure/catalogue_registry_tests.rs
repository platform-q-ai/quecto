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

    let models = source.load_valid_descriptors().unwrap();

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

    let models = source.load_valid_descriptors().unwrap();

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
    let models = source.load_valid_descriptors().unwrap();

    let anthropic = models
        .iter()
        .find(|m| m.qualified_id() == "anthropic-oauth/claude")
        .unwrap();
    assert_eq!(
        anthropic.auth,
        AuthIdentity::OAuth {
            provider: Some(crate::domain::catalogue::ProviderId::new("anthropic").unwrap())
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

#[test]
fn registry_source_reports_malformed_models_json_instead_of_publishing_partial_catalogue() {
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("models.json");
    std::fs::write(&path, "{ not json").unwrap();

    let error = match ModelRegistryCatalogueSource::load_from_path(&path) {
        Ok(_) => panic!("malformed models.json must not load as a catalogue source"),
        Err(error) => error,
    };

    assert!(!error.is_empty());
}

#[test]
fn builtin_and_user_layers_load_independently_for_application_precedence() {
    use crate::catalogue_app::CatalogueSource;

    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"fireworks":{"api":"openai-completions","baseUrl":"https://e.example/v1","auth":{"mode":"apiKey","apiKey":"k"},"models":[{"id":"only-user"}]}}}"#,
    )
    .unwrap();

    let builtin = BuiltinCatalogueSource;
    let user = UserModelsJsonCatalogueSource::from_base_dir(tmp.path());
    assert_eq!(builtin.id(), "builtin");
    assert_eq!(user.id(), "models.json");

    let builtin_models = builtin.load().unwrap();
    let user_models = user.load().unwrap();

    assert!(
        !builtin_models.is_empty(),
        "the built-in layer must carry shipped metadata"
    );
    // The user layer is parsed on its own: it must not carry the built-in
    // entries, because precedence between the layers is the application's.
    assert_eq!(
        user_models
            .iter()
            .map(|model| model.qualified_id())
            .collect::<Vec<_>>(),
        ["fireworks/only-user"]
    );
}

#[test]
fn user_layer_reports_malformed_models_json_and_is_empty_without_the_file() {
    use crate::catalogue_app::CatalogueSource;

    let tmp = tempfile::TempDir::new().unwrap();
    let source = UserModelsJsonCatalogueSource::from_base_dir(tmp.path());
    assert!(source.load().unwrap().is_empty());

    std::fs::write(tmp.path().join("models.json"), "{ not json").unwrap();
    let error = match source.load() {
        Ok(_) => panic!("a malformed user layer must not resolve"),
        Err(error) => error,
    };
    assert!(!error.is_empty());
}

#[test]
fn an_oauth_entry_without_a_named_provider_reports_none_rather_than_the_key() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"anthropic-oauth":{"api":"anthropic-messages","auth":{"mode":"oauth"},"models":[{"id":"claude"}]}}}"#,
    )
    .unwrap();
    let source =
        ModelRegistryCatalogueSource::load_from_path(&tmp.path().join("models.json")).unwrap();

    let model = source
        .load_valid_descriptors()
        .unwrap()
        .into_iter()
        .find(|model| model.qualified_id() == "anthropic-oauth/claude")
        .expect("the entry is listed");

    // The entry is misconfigured; projecting the provider key as if it had been
    // declared would hide that from a client checking for a named provider.
    assert_eq!(model.auth, AuthIdentity::OAuth { provider: None });
}
