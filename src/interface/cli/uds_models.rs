use super::uds::DispatchCtx;

pub(super) fn list_models_response(ctx: &DispatchCtx<'_>) -> serde_json::Value {
    let path = ctx.base_dir.join("models.json");
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
