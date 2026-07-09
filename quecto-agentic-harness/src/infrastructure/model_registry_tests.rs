use super::{AuthMode, ModelRegistry, ProviderApi, resolve_registry_value};

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
        r#"{"providers":{"openai-api":{"api":"openai-completions","models":[{"id":"gpt-5.5","name":"Custom GPT","contextWindow":42}]}}}"#,
    )
    .unwrap();

    let registry = ModelRegistry::load_from_path(&path).unwrap();
    let model = registry.find("openai-api", "gpt-5.5").unwrap();
    assert_eq!(model.display_name.as_deref(), Some("Custom GPT"));
    assert_eq!(model.context_window, 42);
}

#[test]
fn registry_missing_file_returns_builtin_models() {
    let tmp = tempfile::tempdir().unwrap();
    let registry = ModelRegistry::load_from_path(&tmp.path().join("missing-models.json")).unwrap();

    assert!(registry.find("anthropic-api", "claude-fable-5").is_some());
    assert!(registry.find("anthropic-oauth", "claude-fable-5").is_some());
    assert_eq!(
        registry
            .find("openai-api", "gpt-5.5-mini")
            .unwrap()
            .qualified_id(),
        "openai-api/gpt-5.5-mini"
    );
}

#[test]
fn builtin_claude_sonnet_5_resolves_for_api_key_and_oauth_with_published_limits() {
    let registry = ModelRegistry::builtin();

    let api_model = registry
        .find("anthropic-api", "claude-sonnet-5")
        .expect("Claude Sonnet 5 API-key model should be built in");
    assert_eq!(api_model.qualified_id(), "anthropic-api/claude-sonnet-5");
    assert_eq!(
        api_model.display_name.as_deref(),
        Some("Claude Sonnet 5 (API key)")
    );
    assert_eq!(api_model.api, ProviderApi::AnthropicMessages);
    assert_eq!(api_model.auth, AuthMode::ApiKey);
    assert!(api_model.oauth_provider.is_none());
    assert_eq!(
        api_model.input,
        vec!["text".to_string(), "image".to_string()]
    );
    assert_eq!(api_model.context_window, 1_000_000);
    assert_eq!(api_model.max_tokens, 128_000);
    assert!(api_model.max_tokens_explicit);
    assert_eq!(api_model.cost.input, 3.0);
    assert_eq!(api_model.cost.output, 15.0);
    assert_eq!(api_model.cost.cache_read, 0.3);
    assert_eq!(api_model.cost.cache_write, 3.75);

    let oauth_model = registry
        .find("anthropic-oauth", "claude-sonnet-5")
        .expect("Claude Sonnet 5 OAuth model should be built in");
    assert_eq!(
        oauth_model.qualified_id(),
        "anthropic-oauth/claude-sonnet-5"
    );
    assert_eq!(
        oauth_model.display_name.as_deref(),
        Some("Claude Sonnet 5 (OAuth)")
    );
    assert_eq!(oauth_model.api, ProviderApi::AnthropicMessages);
    assert_eq!(oauth_model.auth, AuthMode::OAuth);
    assert_eq!(oauth_model.oauth_provider.as_deref(), Some("anthropic"));
    assert_eq!(
        oauth_model.input,
        vec!["text".to_string(), "image".to_string()]
    );
    assert_eq!(oauth_model.context_window, 1_000_000);
    assert_eq!(oauth_model.max_tokens, 128_000);
    assert!(oauth_model.max_tokens_explicit);
    assert_eq!(oauth_model.cost.input, 3.0);
    assert_eq!(oauth_model.cost.output, 15.0);
    assert_eq!(oauth_model.cost.cache_read, 0.3);
    assert_eq!(oauth_model.cost.cache_write, 3.75);
}

#[test]
fn builtin_gpt_5_6_tiers_resolve_for_api_key_and_oauth_with_published_limits() {
    let registry = ModelRegistry::builtin();
    // (id, input $/1M, output $/1M)
    let tiers = [
        ("gpt-5.6-sol", 5.0, 30.0),
        ("gpt-5.6-terra", 2.5, 15.0),
        ("gpt-5.6-luna", 1.0, 6.0),
    ];
    for provider in ["openai-api", "openai-oauth"] {
        for (id, input, output) in tiers {
            let m = registry
                .find(provider, id)
                .unwrap_or_else(|| panic!("{provider}/{id} should be built in"));
            assert_eq!(m.api, ProviderApi::OpenAiCompletions);
            // Published limits (OpenAI, 2026-07-09): shared across the tiers.
            assert_eq!(m.context_window, 1_050_000, "{id} context window");
            assert!(m.context_window_explicit, "{id} context window is explicit");
            assert_eq!(m.max_tokens, 128_000, "{id} max output");
            assert!(m.max_tokens_explicit, "{id} max output is explicit");
            assert!(m.reasoning, "{id} is a reasoning model");
            // Per-tier pricing; cache read 0.10x input, cache write 1.25x input.
            assert_eq!(m.cost.input, input, "{id} input price");
            assert_eq!(m.cost.output, output, "{id} output price");
            assert_eq!(m.cost.cache_read, input * 0.10, "{id} cache-read price");
            assert_eq!(m.cost.cache_write, input * 1.25, "{id} cache-write price");
        }
    }
    // Auth modes are wired per listing.
    assert_eq!(
        registry.find("openai-api", "gpt-5.6-sol").unwrap().auth,
        AuthMode::ApiKey
    );
    let oauth = registry.find("openai-oauth", "gpt-5.6-sol").unwrap();
    assert_eq!(oauth.auth, AuthMode::OAuth);
    assert_eq!(oauth.oauth_provider.as_deref(), Some("openai"));
}

#[test]
fn builtin_claude_sonnet_5_is_ordered_before_sonnet_4_6_for_each_auth_mode() {
    let registry = ModelRegistry::builtin();
    for provider in ["anthropic-api", "anthropic-oauth"] {
        let sonnet_5 = registry
            .models()
            .iter()
            .position(|m| m.provider == provider && m.id == "claude-sonnet-5")
            .unwrap_or_else(|| panic!("missing {provider}/claude-sonnet-5"));
        let sonnet_4_6 = registry
            .models()
            .iter()
            .position(|m| m.provider == provider && m.id == "claude-sonnet-4-6")
            .unwrap_or_else(|| panic!("missing {provider}/claude-sonnet-4-6"));
        assert!(
            sonnet_5 < sonnet_4_6,
            "{provider}/claude-sonnet-5 should appear before claude-sonnet-4-6"
        );
    }
}

#[test]
fn max_tokens_for_returns_claude_sonnet_5_output_cap_for_both_auth_modes() {
    let registry = ModelRegistry::builtin();
    assert_eq!(
        registry.max_tokens_for("anthropic-api/claude-sonnet-5"),
        Some(128_000)
    );
    assert_eq!(
        registry.max_tokens_for("anthropic-oauth/claude-sonnet-5"),
        Some(128_000)
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

#[test]
fn registry_defaults_auth_mode_to_api_key_when_key_present() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("models.json");
    std::fs::write(
        &path,
        r#"{"providers":{"fireworks":{"api":"openai-completions","baseUrl":"https://api.fireworks.ai/inference/v1","apiKey":"sk-fw","models":[{"id":"m"}]}}}"#,
    )
    .unwrap();

    let registry = ModelRegistry::load_from_path(&path).unwrap();
    let model = registry.find("fireworks", "m").unwrap();
    assert_eq!(model.auth, AuthMode::ApiKey);
    assert!(model.oauth_provider.is_none());
}

#[test]
fn registry_parses_explicit_oauth_auth_block() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("models.json");
    std::fs::write(
        &path,
        r#"{
          "providers": {
            "anthropic-oauth": {
              "api": "anthropic-messages",
              "auth": { "mode": "oauth", "oauthProvider": "anthropic" },
              "models": [{ "id": "claude-opus-4-8" }]
            }
          }
        }"#,
    )
    .unwrap();

    let registry = ModelRegistry::load_from_path(&path).unwrap();
    let model = registry.find("anthropic-oauth", "claude-opus-4-8").unwrap();
    assert_eq!(model.auth, AuthMode::OAuth);
    assert_eq!(model.oauth_provider.as_deref(), Some("anthropic"));
}

#[test]
fn registry_parses_explicit_api_key_auth_block() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("models.json");
    std::fs::write(
        &path,
        r#"{
          "providers": {
            "anthropic-api": {
              "api": "anthropic-messages",
              "baseUrl": "https://api.anthropic.com",
              "auth": { "mode": "apiKey", "apiKey": "sk-ant-direct" },
              "models": [{ "id": "claude-opus-4-8" }]
            }
          }
        }"#,
    )
    .unwrap();

    let registry = ModelRegistry::load_from_path(&path).unwrap();
    let model = registry.find("anthropic-api", "claude-opus-4-8").unwrap();
    assert_eq!(model.auth, AuthMode::ApiKey);
    assert_eq!(model.api_key.as_deref(), Some("sk-ant-direct"));
    assert!(model.oauth_provider.is_none());
}

#[test]
fn registry_rejects_unknown_auth_mode() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("models.json");
    std::fs::write(
        &path,
        r#"{"providers":{"x":{"api":"openai-completions","auth":{"mode":"vault"},"models":[{"id":"m"}]}}}"#,
    )
    .unwrap();

    let err = ModelRegistry::load_from_path(&path)
        .unwrap_err()
        .to_string();
    assert!(err.contains("unknown auth mode 'vault'"), "{err}");
}

#[serial_test::serial]
#[test]
fn registry_oauth_auth_block_top_level_api_key_resolves_from_env() {
    // The provider-level apiKey in an explicit apiKey auth block supports env
    // interpolation just like the legacy top-level apiKey field.
    // SAFETY: this unit test mutates a unique process environment variable; no other test uses this key.
    unsafe {
        std::env::set_var("QUECTO_TEST_AUTH_BLOCK_KEY", "sk-resolved");
    }
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("models.json");
    std::fs::write(
        &path,
        r#"{"providers":{"x":{"api":"openai-completions","baseUrl":"https://e.example/v1","auth":{"mode":"apiKey","apiKey":"$QUECTO_TEST_AUTH_BLOCK_KEY"},"models":[{"id":"m"}]}}}"#,
    )
    .unwrap();

    let registry = ModelRegistry::load_from_path(&path).unwrap();
    let model = registry.find("x", "m").unwrap();
    assert_eq!(model.api_key.as_deref(), Some("sk-resolved"));
    // SAFETY: paired cleanup for the unique test env var set above.
    unsafe {
        std::env::remove_var("QUECTO_TEST_AUTH_BLOCK_KEY");
    }
}

// === #935: per-model output-cap lookup (the seam that feeds the clamp) ===

#[test]
fn max_tokens_for_returns_model_cap_for_known_model() {
    // This is the registry-lookup seam that the agent build and the set_model
    // path use to re-derive a model's output cap. A lower-cap model (e.g.
    // Fireworks qwen3p7-plus = 65536) must report its real cap so the clamp
    // can apply min(configured, cap).
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("models.json");
    std::fs::write(
        &path,
        r#"{"providers":{"fireworks":{"api":"openai-completions","baseUrl":"https://e.example/v1","apiKey":"k","models":[{"id":"qwen3p7-plus","maxTokens":65536}]}}}"#,
    )
    .unwrap();

    let registry = ModelRegistry::load_from_path(&path).unwrap();
    assert_eq!(
        registry.max_tokens_for("fireworks/qwen3p7-plus"),
        Some(65_536)
    );
}

#[test]
fn max_tokens_for_returns_none_for_unknown_or_unqualified_model() {
    let registry = ModelRegistry::builtin();
    assert_eq!(registry.max_tokens_for("fireworks/does-not-exist"), None);
    // Not provider/id-shaped → no clamp.
    assert_eq!(registry.max_tokens_for("bare-model-name"), None);
}

#[test]
fn max_tokens_for_returns_none_when_model_omits_max_tokens() {
    // A model that exists in models.json but does NOT declare `maxTokens` has no
    // real output limit known to the registry. It must return None (no clamp),
    // not the synthesized 16_384 default — otherwise a user whose configured
    // max_tokens exceeds 16_384 would have every request silently over-clamped.
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("models.json");
    std::fs::write(
        &path,
        r#"{"providers":{"fireworks":{"api":"openai-completions","baseUrl":"https://e.example/v1","apiKey":"k","models":[{"id":"no-cap-model"}]}}}"#,
    )
    .unwrap();

    let registry = ModelRegistry::load_from_path(&path).unwrap();
    // The field still carries the synthesized default for display purposes...
    assert_eq!(
        registry
            .find("fireworks", "no-cap-model")
            .unwrap()
            .max_tokens,
        16_384
    );
    // ...but the clamp lookup must treat an absent cap as "unknown".
    assert_eq!(registry.max_tokens_for("fireworks/no-cap-model"), None);
}

#[test]
fn model_limits_from_base_dir_reads_output_cap_from_models_json() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"fireworks":{"api":"openai-completions","baseUrl":"https://e.example/v1","apiKey":"k","models":[{"id":"qwen3p7-plus","maxTokens":65536}]}}}"#,
    )
    .unwrap();

    let (cap, window) =
        ModelRegistry::model_limits_from_base_dir(tmp.path(), "fireworks/qwen3p7-plus");
    assert_eq!(cap, Some(65_536));
    assert_eq!(window, None, "no declared window must not clamp");
}

// --- #1044: known context windows for the window-aware budget ---

#[test]
fn context_window_for_returns_declared_windows_only() {
    let registry = ModelRegistry::builtin();
    // A builtin model with an explicitly declared window resolves to it.
    assert_eq!(
        registry.context_window_for("anthropic-api/claude-sonnet-5"),
        Some(1_000_000),
        "a declared context window must be resolvable by qualified model id"
    );
    // A model whose window is only the synthesized default is "unknown":
    // it must not clamp the configured budget.
    assert_eq!(
        registry.context_window_for("anthropic-api/claude-opus-4-8"),
        None
    );
    // Unknown models and non-qualified ids are unknown.
    assert_eq!(registry.context_window_for("nope/never-heard-of-it"), None);
    assert_eq!(registry.context_window_for("not-qualified"), None);
}

#[test]
fn model_limits_from_base_dir_reads_context_window_from_models_json() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"fireworks":{"api":"openai-completions","baseUrl":"https://e.example/v1","apiKey":"k","models":[{"id":"small-window","contextWindow":32768},{"id":"no-window"}]}}}"#,
    )
    .unwrap();

    assert_eq!(
        ModelRegistry::model_limits_from_base_dir(tmp.path(), "fireworks/small-window").1,
        Some(32_768)
    );
    assert_eq!(
        ModelRegistry::model_limits_from_base_dir(tmp.path(), "fireworks/no-window").1,
        None,
        "a listed model without a declared window must not clamp"
    );
}
