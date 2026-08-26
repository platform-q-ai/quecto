//! UDS `list_models` read path (epic #1193, slice 2).
//!
//! Reads the published catalogue snapshot through the query use case — never
//! parsing `models.json` itself — and renders the legacy wire shape from the
//! shared model-listing projection plus the snapshot entries. The TUI model
//! list is a projection of this same response, so CLI, UDS, and TUI all render
//! one snapshot generation.

use crate::application::catalogue::{
    CatalogueQuery, ModelListing, QueryCatalogueUseCase, project_model_listing,
};
use crate::domain::catalogue::{AuthIdentity, CatalogueSnapshot, TransportKind};

use super::uds::DispatchCtx;

pub(super) fn list_models_response(ctx: &DispatchCtx<'_>) -> serde_json::Value {
    list_models_data(ctx.base_dir)
}

pub fn list_models_data(base_dir: &std::path::Path) -> serde_json::Value {
    // Re-resolving on each request keeps the listing level with on-disk edits
    // exactly as the legacy per-request parse did; explicit refresh replaces
    // this in epic #1193 slices 4-5. The read below still goes through the
    // published snapshot only.
    let (store, resolved) = super::catalogue_bridge::resolve_and_publish_for(base_dir);
    if let Some(error) = resolved.source_errors.first() {
        // Legacy parity: a malformed models.json returned no models plus an
        // error, rather than silently listing a catalogue the user's file no
        // longer matches. The last valid snapshot stays published regardless.
        return serde_json::json!({ "models": [], "error": error.error });
    }
    let snapshot = QueryCatalogueUseCase::new(store).query(CatalogueQuery::All);
    let listing = project_model_listing(&snapshot);
    render_listing(&listing, &snapshot)
}

/// Render the legacy `list_models` wire shape. Identity, display name, and
/// configured status come from the shared projection rows; transport, auth,
/// and capability metadata come from the matching snapshot entries.
fn render_listing(listing: &ModelListing, snapshot: &CatalogueSnapshot) -> serde_json::Value {
    serde_json::json!({
        "generation": listing.generation,
        "models": listing.rows.iter().zip(snapshot.entries()).map(|(row, entry)| {
            serde_json::json!({
                "provider": entry.reference().provider().as_str(),
                "id": entry.reference().model().as_str(),
                "model": row.qualified_id,
                "name": row.display_name,
                "api": match entry.provider.transport {
                    TransportKind::OpenAiCompletions => "openai-completions",
                    TransportKind::AnthropicMessages => "anthropic-messages",
                    TransportKind::GoogleGenerativeAi => "google-generative-ai",
                },
                "auth": match entry.provider.auth {
                    AuthIdentity::ApiKey => "apiKey",
                    AuthIdentity::OAuth { .. } => "oauth",
                },
                "oauthProvider": entry.provider.auth.oauth_provider().map(|p| p.as_str()),
                "contextWindow": entry.model.capabilities.context_window,
                "maxTokens": entry.model.capabilities.max_output_tokens,
                "input": entry.model.capabilities.input_modalities,
                "cost": {
                    "input": entry.model.capabilities.cost.input,
                    "output": entry.model.capabilities.cost.output,
                    "cacheRead": entry.model.capabilities.cost.cache_read,
                    "cacheWrite": entry.model.capabilities.cost.cache_write,
                },
                "reasoning": entry.model.capabilities.reasoning,
                "configured": row.runnable,
            })
        }).collect::<Vec<_>>()
    })
}

#[cfg(test)]
#[path = "uds_models_tests.rs"]
mod tests;
