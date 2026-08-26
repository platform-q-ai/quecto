//! UDS `list_models` read path (epic #1193, slice 2).
//!
//! Reads the published catalogue snapshot through the query use case — never
//! parsing `models.json` itself — and renders the legacy wire shape from the
//! shared model-listing projection plus the snapshot entries. The TUI model
//! list is a projection of this same response, so CLI, UDS, and TUI all render
//! one snapshot generation.

use crate::application::catalogue::ResolvedCatalogue;
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
    render_listing(&listing, &snapshot, &resolved)
}

/// Diagnostics for records the catalogue dropped: entries the domain rejected
/// after validation plus records a source could not map at all. Rendered on
/// the wire so a model that vanishes from the listing is never silent.
fn rejected_diagnostics(resolved: &ResolvedCatalogue) -> Vec<serde_json::Value> {
    resolved
        .rejected
        .iter()
        .map(|rejection| {
            serde_json::json!({
                "model": rejection.entry.reference().qualified_id(),
                "reason": rejection.error.to_string(),
            })
        })
        .chain(resolved.skipped.iter().map(|(source, record)| {
            serde_json::json!({
                "model": record.record,
                "reason": format!("{source}: {}", record.error),
            })
        }))
        .collect()
}

/// Render the legacy `list_models` wire shape. Identity, display name, and
/// configured status come from the shared projection rows; transport, auth,
/// and capability metadata come from the matching snapshot entries.
fn render_listing(
    listing: &ModelListing,
    snapshot: &CatalogueSnapshot,
    resolved: &ResolvedCatalogue,
) -> serde_json::Value {
    // Rows are joined to snapshot entries by qualified id, not by position,
    // so a projection that ever filters or reorders cannot silently pair one
    // model's identity with another model's metadata.
    let rows: std::collections::HashMap<&str, &crate::application::catalogue::ModelListingRow> =
        listing
            .rows
            .iter()
            .map(|row| (row.qualified_id.as_str(), row))
            .collect();
    serde_json::json!({
        "generation": listing.generation,
        "rejected": rejected_diagnostics(resolved),
        "models": snapshot.entries().iter().filter_map(|entry| {
            let row = rows.get(entry.reference().qualified_id().as_str())?;
            Some(serde_json::json!({
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
            }))
        }).collect::<Vec<_>>()
    })
}

/// UDS `refresh_models` operation (epic #1193, slice 4): drive the one
/// application refresh use case and render per-source outcomes on the wire.
/// The dispatch loop runs this on a blocking worker thread (see
/// `dispatch_fieldless_command`), so other UDS commands stay serviced while
/// a refresh is in flight; the per-source budget is still kept tight so an
/// unattended refresh converges quickly.
pub fn refresh_models_data(base_dir: &std::path::Path, source: Option<&str>) -> serde_json::Value {
    use crate::application::catalogue_refresh::{
        RefreshBounds, RefreshSelection, SourceRefreshStatus,
    };
    let selection = match source {
        Some(name) => RefreshSelection::Only(vec![name.to_string()]),
        None => RefreshSelection::All,
    };
    let bounds = RefreshBounds {
        timeout: std::time::Duration::from_secs(4),
        ..RefreshBounds::default()
    };
    let report = super::catalogue_refresh_bridge::refresh_catalogue(base_dir, &selection, bounds);
    let outcomes: Vec<serde_json::Value> = report
        .outcomes
        .iter()
        .map(|outcome| {
            let (status, models, reason) = match &outcome.status {
                SourceRefreshStatus::Updated { models } => ("updated", Some(*models), None),
                SourceRefreshStatus::Unchanged { models } => ("unchanged", Some(*models), None),
                SourceRefreshStatus::Unsupported { reason } => {
                    ("unsupported", None, Some(reason.clone()))
                }
                SourceRefreshStatus::Failed { reason } => ("failed", None, Some(reason.clone())),
                SourceRefreshStatus::Cancelled => ("cancelled", None, None),
            };
            serde_json::json!({
                "source": outcome.source,
                "status": status,
                "models": models,
                "reason": reason,
            })
        })
        .collect();
    serde_json::json!({
        "outcomes": outcomes,
        "generation": report
            .resolved
            .as_ref()
            .map(|resolved| resolved.snapshot.generation()),
    })
}

#[cfg(test)]
#[path = "uds_models_tests.rs"]
mod tests;
