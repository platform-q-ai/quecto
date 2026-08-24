use crate::application::catalogue::{CatalogueSource, ResolveCatalogueUseCase};
use crate::domain::catalogue::{AuthIdentity, TransportKind};
use crate::infrastructure::catalogue_registry::ModelRegistryCatalogueSource;

use super::uds::DispatchCtx;

pub(super) fn list_models_response(ctx: &DispatchCtx<'_>) -> serde_json::Value {
    list_models_data(ctx.base_dir)
}

fn list_models_data(base_dir: &std::path::Path) -> serde_json::Value {
    match load_catalogue_descriptors(base_dir) {
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

fn load_catalogue_descriptors(
    base_dir: &std::path::Path,
) -> Result<Vec<crate::domain::catalogue::ModelDescriptor>, String> {
    let path = base_dir.join("models.json");
    let source = ModelRegistryCatalogueSource::load_from_path(&path)?;
    let resolver = ResolveCatalogueUseCase::new(vec![&source as &dyn CatalogueSource]);
    Ok(resolver.resolve(0)?.models().to_vec())
}

#[cfg(test)]
#[path = "uds_models_tests.rs"]
mod tests;
