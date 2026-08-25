use crate::domain::catalogue::{AuthIdentity, CatalogueSnapshot, TransportKind};

use super::uds::DispatchCtx;

pub(super) fn list_models_response(ctx: &DispatchCtx<'_>) -> serde_json::Value {
    list_catalogue_data(&ctx.agent.catalogue)
}

fn list_catalogue_data(catalogue: &CatalogueSnapshot) -> serde_json::Value {
    list_models_slice(catalogue.models())
}

#[cfg(test)]
fn list_models_data(
    provider: &std::sync::Arc<dyn crate::domain::provider::LlmProvider>,
) -> serde_json::Value {
    list_models_slice(provider.model_descriptors().unwrap_or(&[]))
}

fn list_models_slice(models: &[crate::domain::catalogue::ModelDescriptor]) -> serde_json::Value {
    serde_json::json!({
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
    })
}

#[cfg(test)]
#[path = "uds_models_tests.rs"]
mod tests;
