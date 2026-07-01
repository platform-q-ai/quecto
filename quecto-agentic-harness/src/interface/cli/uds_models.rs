use super::uds::DispatchCtx;

pub(super) fn list_models_response(ctx: &DispatchCtx<'_>) -> serde_json::Value {
    list_models_data(ctx.base_dir)
}

pub(crate) fn list_models_data(base_dir: &std::path::Path) -> serde_json::Value {
    let path = base_dir.join("models.json");
    match crate::infrastructure::model_registry::ModelRegistry::load_from_path(&path) {
        Ok(registry) => serde_json::json!({
            "models": registry.models().iter().map(|m| serde_json::json!({
                "provider": m.provider,
                "id": m.id,
                "model": m.qualified_id(),
                "name": m.display_name,
                "api": match m.api {
                    crate::infrastructure::model_registry::ProviderApi::OpenAiCompletions => "openai-completions",
                    crate::infrastructure::model_registry::ProviderApi::AnthropicMessages => "anthropic-messages",
                    crate::infrastructure::model_registry::ProviderApi::GoogleGenerativeAi => "google-generative-ai",
                },
                "auth": match m.auth {
                    crate::infrastructure::model_registry::AuthMode::ApiKey => "apiKey",
                    crate::infrastructure::model_registry::AuthMode::OAuth => "oauth",
                },
                "oauthProvider": m.oauth_provider,
                "contextWindow": m.context_window,
                "maxTokens": m.max_tokens,
                "input": m.input,
                "cost": {
                    "input": m.cost.input,
                    "output": m.cost.output,
                    "cacheRead": m.cost.cache_read,
                    "cacheWrite": m.cost.cache_write,
                },
                "reasoning": m.reasoning,
                "configured": m.api_key.as_deref().is_some_and(|k| !k.is_empty()) || m.base_url.is_some(),
            })).collect::<Vec<_>>()
        }),
        Err(err) => serde_json::json!({ "models": [], "error": err.to_string() }),
    }
}

#[cfg(test)]
mod tests {
    use super::list_models_data;

    #[test]
    fn list_models_data_serializes_registry_models() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("models.json"),
            r#"{
              "providers": {
                "anthropicish": {
                  "api": "anthropic-messages",
                  "apiKey": "sk-a",
                  "models": [{"id":"claude-custom", "reasoning": true}]
                },
                "googleish": {
                  "api": "google-generative-ai",
                  "baseUrl": "https://google.example/v1",
                  "models": [{"id":"gemini-custom"}]
                },
                "openish": {
                  "api": "openai-completions",
                  "models": [{"id":"open-custom", "cost":{"input":1.0,"output":2.0,"cacheRead":3.0,"cacheWrite":4.0}}]
                }
              }
            }"#,
        )
        .unwrap();

        let data = list_models_data(tmp.path());
        let models = data["models"].as_array().unwrap();
        let find = |id: &str| {
            models
                .iter()
                .find(|m| m["model"].as_str().unwrap().ends_with(id))
                .unwrap()
        };

        let anthropic = find("claude-custom");
        assert_eq!(anthropic["api"], "anthropic-messages");
        assert_eq!(anthropic["auth"], "apiKey");
        assert_eq!(anthropic["configured"], true);
        assert_eq!(anthropic["reasoning"], true);

        let google = find("gemini-custom");
        assert_eq!(google["api"], "google-generative-ai");
        assert_eq!(google["configured"], true);

        let open = find("open-custom");
        assert_eq!(open["api"], "openai-completions");
        assert_eq!(open["configured"], false);
        assert_eq!(open["cost"]["cacheRead"], 3.0);
        assert_eq!(open["cost"]["cacheWrite"], 4.0);
    }

    #[test]
    fn models_command_lists_claude_sonnet_5_builtins() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = crate::interface::cli::CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };

        let out =
            crate::interface::cli::run_with_output(vec!["quecto".into(), "models".into()], &ctx);

        assert_eq!(out.exit_code, 0, "stderr: {}", out.stderr);
        assert!(out.stderr.is_empty());
        let data: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
        let models = data["models"].as_array().unwrap();
        assert!(models.iter().any(|m| {
            m["model"] == "anthropic-api/claude-sonnet-5"
                && m["contextWindow"] == 1_000_000
                && m["maxTokens"] == 128_000
        }));
        assert!(models.iter().any(|m| {
            m["model"] == "anthropic-oauth/claude-sonnet-5"
                && m["contextWindow"] == 1_000_000
                && m["maxTokens"] == 128_000
        }));
    }

    #[test]
    fn list_models_data_reports_registry_errors() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("models.json"), "not json").unwrap();

        let data = list_models_data(tmp.path());

        assert_eq!(data["models"].as_array().unwrap().len(), 0);
        assert!(data["error"].as_str().unwrap().contains("failed to parse"));
    }
}
