use crate::domain::catalogue::{AuthIdentity, ModelDescriptor, TransportKind};
use crate::infrastructure::providers::retry::RetryingProvider;
use crate::infrastructure::providers::router::ProviderRouter;

use super::uds::DispatchCtx;

pub(super) fn list_models_response(ctx: &DispatchCtx<'_>) -> serde_json::Value {
    list_models_data(&ctx.agent.provider)
}

fn list_models_data(
    provider: &std::sync::Arc<dyn crate::domain::provider::LlmProvider>,
) -> serde_json::Value {
    match runtime_catalogue_descriptors(provider) {
        Ok(models) => serde_json::json!({
            "models": models.iter().map(|m| serde_json::json!({
                "provider": m.reference.provider().as_str(),
                "id": m.reference.model().as_str(),
                "model": m.qualified_id(),
                "name": m.display_name,
                "api": match m.transport {
                    TransportKind::OpenAiCompletions => "openai-completions",
                    TransportKind::AnthropicMessages => "anthropic-messages",
                    TransportKind::GoogleGenerativeAi => "google-generative-ai",
                },
                "auth": match m.auth {
                    AuthIdentity::ApiKey => "apiKey",
                    AuthIdentity::OAuth { .. } => "oauth",
                },
                "oauthProvider": m.auth.oauth_provider().map(|provider| provider.as_str()),
                "contextWindow": m.capabilities.context_window,
                "maxTokens": m.capabilities.max_tokens,
                "input": m.capabilities.input,
                "cost": {
                    "input": m.capabilities.cost.input,
                    "output": m.capabilities.cost.output,
                    "cacheRead": m.capabilities.cost.cache_read,
                    "cacheWrite": m.capabilities.cost.cache_write,
                },
                "reasoning": m.capabilities.reasoning,
                "configured": m.configured,
            })).collect::<Vec<_>>()
        }),
        Err(err) => serde_json::json!({ "models": [], "error": err }),
    }
}

fn runtime_catalogue_descriptors(
    provider: &std::sync::Arc<dyn crate::domain::provider::LlmProvider>,
) -> Result<Vec<ModelDescriptor>, String> {
    let provider = provider
        .as_any()
        .downcast_ref::<RetryingProvider>()
        .map_or(provider, RetryingProvider::inner);
    if let Some(models) = provider.model_descriptors() {
        if !models.is_empty() {
            return Ok(models.to_vec());
        }
    }
    let Some(router) = provider.as_any().downcast_ref::<ProviderRouter>() else {
        return Ok(Vec::new());
    };
    let mut models = Vec::new();
    for provider in router.providers() {
        if let Some(descriptors) = provider.model_descriptors() {
            models.extend_from_slice(descriptors);
        }
    }
    Ok(models)
}

#[cfg(test)]
#[path = "uds_models_tests.rs"]
mod tests;
