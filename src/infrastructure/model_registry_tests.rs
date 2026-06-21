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
    assert!(!model.allow_remote_http);
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

#[test]
fn registry_missing_file_returns_builtin_models() {
    let tmp = tempfile::tempdir().unwrap();
    let registry = ModelRegistry::load_from_path(&tmp.path().join("missing-models.json")).unwrap();

    assert!(registry.find("anthropic", "claude-fable-5").is_some());
    assert_eq!(
        registry
            .find("openai", "gpt-5.5-mini")
            .unwrap()
            .qualified_id(),
        "openai/gpt-5.5-mini"
    );
}

#[test]
fn registry_loads_all_protocols_and_model_overrides() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("models.json");
    std::fs::write(
        &path,
        r#"{
          "providers": {
            "anthropicish": {
              "apiBase": "https://anthropic.example/v1",
              "apiKey": "sk-anthropic",
              "api": "anthropic-messages",
              "authHeader": false,
              "allowRemoteHttp": true,
              "models": [{
                "id": "claude-custom",
                "name": "Claude Custom",
                "reasoning": true,
                "input": ["text", "image"],
                "contextWindow": 200000,
                "maxTokens": 32000,
                "cost": { "input": 1.25, "output": 2.5, "cacheRead": 0.1, "cacheWrite": 0.2 }
              }]
            },
            "googleish": {
              "api": "google-generative-ai",
              "models": [{
                "id": "gemini-custom",
                "cost": { "cache_read": 0.3, "cache_write": 0.4 }
              }]
            },
            "defaultish": {
              "models": [{ "id": "openai-default-api" }]
            }
          }
        }"#,
    )
    .unwrap();

    let registry = ModelRegistry::load_from_path(&path).unwrap();
    let anthropic = registry.find("anthropicish", "claude-custom").unwrap();
    assert_eq!(anthropic.api, ProviderApi::AnthropicMessages);
    assert_eq!(
        anthropic.base_url.as_deref(),
        Some("https://anthropic.example/v1")
    );
    assert_eq!(anthropic.api_key.as_deref(), Some("sk-anthropic"));
    assert!(!anthropic.auth_header);
    assert!(anthropic.allow_remote_http);
    assert_eq!(
        anthropic.input,
        vec!["text".to_string(), "image".to_string()]
    );
    assert_eq!(anthropic.context_window, 200000);
    assert_eq!(anthropic.max_tokens, 32000);
    assert!(anthropic.reasoning);
    assert_eq!(anthropic.cost.input, 1.25);
    assert_eq!(anthropic.cost.output, 2.5);
    assert_eq!(anthropic.cost.cache_read, 0.1);
    assert_eq!(anthropic.cost.cache_write, 0.2);

    let google = registry.find("googleish", "gemini-custom").unwrap();
    assert_eq!(google.api, ProviderApi::GoogleGenerativeAi);
    assert_eq!(google.cost.cache_read, 0.3);
    assert_eq!(google.cost.cache_write, 0.4);

    let default_api = registry.find("defaultish", "openai-default-api").unwrap();
    assert_eq!(default_api.api, ProviderApi::OpenAiCompletions);
}

#[test]
fn registry_reports_parse_and_read_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let bad_json = tmp.path().join("bad.json");
    std::fs::write(&bad_json, "not json").unwrap();
    assert!(
        ModelRegistry::load_from_path(&bad_json)
            .unwrap_err()
            .to_string()
            .contains("failed to parse models registry")
    );

    assert!(
        ModelRegistry::load_from_path(tmp.path())
            .unwrap_err()
            .to_string()
            .contains("failed to read models registry")
    );
}

#[test]
fn registry_value_resolution_handles_missing_and_literal_dollars() {
    let missing = |_name: &str| None::<String>;

    assert_eq!(resolve_registry_value("$MISSING", missing), "");
    assert_eq!(resolve_registry_value("${MISSING", missing), "${MISSING");
    assert_eq!(resolve_registry_value("cost is $5", missing), "cost is ");
    assert_eq!(resolve_registry_value("plain", missing), "plain");
}
