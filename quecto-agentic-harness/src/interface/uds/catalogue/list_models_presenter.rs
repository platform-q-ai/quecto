//! Wire rendering of the `list_models` outcome (#1845): the legacy response
//! shape the CLI, UDS clients and the TUI model list all read. Identity,
//! transport, auth and capability metadata come straight from the listed
//! domain entries; `configured` is the use case's runnable verdict.

use crate::application::catalogue::dto::{ModelCatalogueListing, ModelListingOutcome};
use crate::domain::catalogue::AuthIdentity;

pub fn render(outcome: &ModelListingOutcome) -> serde_json::Value {
    match outcome {
        // Legacy wire parity: no models plus the first failed source's error,
        // rather than silently listing a catalogue the user's inputs no
        // longer match. The DTO carries every failed source; the legacy
        // shape has one `error` field.
        ModelListingOutcome::SourcesUnavailable(errors) => {
            let first = errors
                .first()
                .map(|error| error.error.as_str())
                .unwrap_or("catalogue sources unavailable");
            serde_json::json!({ "models": [], "error": first })
        }
        ModelListingOutcome::Listed(listing) => render_listing(listing),
    }
}

fn render_listing(listing: &ModelCatalogueListing) -> serde_json::Value {
    serde_json::json!({
        "generation": listing.generation,
        "rejected": listing.rejected.iter().map(|diagnostic| serde_json::json!({
            "model": diagnostic.model,
            "reason": diagnostic.reason,
        })).collect::<Vec<_>>(),
        "models": listing.models.iter().map(|listed| {
            let entry = &listed.entry;
            serde_json::json!({
                "provider": entry.reference().provider().as_str(),
                "id": entry.reference().model().as_str(),
                "model": entry.reference().qualified_id(),
                "name": entry.model.display_name,
                "api": entry.provider.transport.stable_id(),
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
                "effortLevels": entry.model.capabilities.effort_levels,
                "configured": listed.runnable,
            })
        }).collect::<Vec<_>>()
    })
}

#[cfg(test)]
#[path = "list_models_presenter_tests.rs"]
mod tests;
