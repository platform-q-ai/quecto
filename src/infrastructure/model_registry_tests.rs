use super::{ModelRegistry, ProviderApi, resolve_registry_value};

#[test]
fn registry_loads_pi_shaped_models_json_with_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("models.json");
    std::fs::write(
        &path,
        r#"{
          "providers": {
            "fireworks": {
              "baseUrl": "https://api.fireworks.ai/inference/v1",
              "apiKey": "$FIREWORKS_API_KEY",
              "api": "openai-completions",
              "authHeader": true,
              "models": [{ "id": "accounts/fireworks/models/glm-5p2", "name": "GLM 5.2" }]
            }
          }
        }"#,
    )
    .unwrap();

    let registry = ModelRegistry::load_from_path(&path).unwrap();
    let model = registry
        .find("fireworks", "accounts/fireworks/models/glm-5p2")
        .expect("fireworks model should be indexed by provider+opaque id");

    assert_eq!(model.provider, "fireworks");
    assert_eq!(model.id, "accounts/fireworks/models/glm-5p2");
    assert_eq!(model.display_name.as_deref(), Some("GLM 5.2"));
    assert_eq!(model.api, ProviderApi::OpenAiCompletions);
    assert_eq!(model.context_window, 128_000);
    assert_eq!(model.max_tokens, 16_384);
    assert_eq!(model.input, vec!["text".to_string()]);
    assert_eq!(model.cost.input, 0.0);
}

#[test]
fn registry_rejects_unknown_wire_protocols() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("models.json");
    std::fs::write(
        &path,
        r#"{"providers":{"x":{"baseUrl":"https://example.com/v1","apiKey":"sk","api":"cohere-chat","models":[{"id":"m"}]}}}"#,
    )
    .unwrap();

    let err = ModelRegistry::load_from_path(&path)
        .unwrap_err()
        .to_string();
    assert!(err.contains("unknown api 'cohere-chat'"), "{err}");
}

#[test]
fn registry_value_resolution_supports_env_and_dollar_escape() {
    let env = |name: &str| match name {
        "FIREWORKS_API_KEY" => Some("sk-env".to_string()),
        _ => None,
    };

    assert_eq!(resolve_registry_value("$FIREWORKS_API_KEY", env), "sk-env");
    assert_eq!(
        resolve_registry_value("${FIREWORKS_API_KEY}", env),
        "sk-env"
    );
    assert_eq!(
        resolve_registry_value("Bearer $FIREWORKS_API_KEY", env),
        "Bearer sk-env"
    );
    assert_eq!(
        resolve_registry_value("$$FIREWORKS_API_KEY", env),
        "$FIREWORKS_API_KEY"
    );
}

#[test]
fn registry_custom_models_override_builtin_by_provider_and_id() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("models.json");
    std::fs::write(
        &path,
        r#"{"providers":{"openai":{"api":"openai-completions","models":[{"id":"gpt-5.5","name":"Custom GPT","contextWindow":42}]}}}"#,
    )
    .unwrap();

    let registry = ModelRegistry::load_from_path(&path).unwrap();
    let model = registry.find("openai", "gpt-5.5").unwrap();
    assert_eq!(model.display_name.as_deref(), Some("Custom GPT"));
    assert_eq!(model.context_window, 42);
}
