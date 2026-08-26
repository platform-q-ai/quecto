//! Interface-level composition of the catalogue refresh path (epic #1193,
//! slice 4): wires the infrastructure discovery adapters into the application
//! refresh use case so the CLI, UDS, and TUI surfaces all drive one refresh
//! operation and own no discovery semantics themselves.

use std::path::Path;

use crate::application::catalogue_refresh::{
    CatalogueRefreshReport, RefreshBounds, RefreshCatalogueSourcesUseCase, RefreshContext,
    RefreshPorts, RefreshSelection, SourceRefreshOutcome, SourceRefreshStatus,
};
use crate::infrastructure::catalogue_discovery::{SecretsRedaction, configured_discovery};
use crate::infrastructure::catalogue_registry::snapshot_store_for;
use crate::interface::cli::catalogue_bridge::CatalogueInputs;

/// Outcome-source id used when the registry file itself (rather than one
/// provider) fails to parse.
pub const REGISTRY_FILE_SOURCE: &str = "models.json";

/// Refresh the configured refreshable catalogue sources for `base_dir` in one
/// operation and republish the effective catalogue through the normal resolve
/// path. Network happens only here — ordinary catalogue reads stay
/// network-free.
pub fn refresh_catalogue(
    base_dir: &Path,
    selection: &RefreshSelection,
    bounds: RefreshBounds,
) -> CatalogueRefreshReport {
    // One models.json read feeds both the refreshable-source enumeration and
    // the post-refresh resolve, so a concurrent edit can never make the
    // refreshed source set and the republished catalogue describe two
    // different files in one report (slice-4 review).
    let inputs = CatalogueInputs::load(base_dir);
    let discovery = match inputs.provider_defaults() {
        Ok(providers) => configured_discovery(base_dir, providers),
        // A catalogue file that cannot be enumerated is one failed outcome,
        // not a crash: the previous valid snapshot stays published.
        Err(error) => {
            return CatalogueRefreshReport {
                outcomes: vec![SourceRefreshOutcome {
                    source: REGISTRY_FILE_SOURCE.to_string(),
                    status: SourceRefreshStatus::Failed {
                        reason: error.clone(),
                    },
                }],
                resolved: None,
            };
        }
    };
    let refreshables: Vec<&dyn crate::application::catalogue_refresh::RefreshableCatalogueSource> =
        discovery.sources.iter().map(AsRef::as_ref).collect();
    // Resolve inputs: `inputs.sources()` already feeds the discovered layer
    // from every persisted discovery cache (caches are read lazily, so they
    // see this run's rewrites); append live sources only for providers whose
    // cache did not exist when the inputs were enumerated (first refresh), so
    // no provider's models are fed in twice. Precedence is layer-based, so
    // the user layers win regardless of input order.
    let cached_providers = inputs.discovered_providers();
    let mut sources = inputs.sources();
    sources.extend(
        discovery
            .sources
            .iter()
            .filter(|s| !cached_providers.contains(&s.id()))
            .map(|s| s.as_ref() as &dyn crate::application::catalogue::CatalogueSource),
    );
    let store = snapshot_store_for(base_dir);
    let redaction = SecretsRedaction::new(discovery.secrets.clone());
    let ports = RefreshPorts {
        refreshables: &refreshables,
        sources: &sources,
        credentials: &inputs.credentials,
        store: &store,
        redaction: &redaction,
    };
    RefreshCatalogueSourcesUseCase.refresh(&ports, selection, &RefreshContext::new(bounds))
}

/// Render one refresh outcome as a human-readable status line.
pub fn describe_outcome(outcome: &SourceRefreshOutcome) -> String {
    match &outcome.status {
        SourceRefreshStatus::Updated { models } => {
            format!("{}: discovered {models} model(s)", outcome.source)
        }
        SourceRefreshStatus::Unchanged { models } => {
            format!("{}: unchanged ({models} model(s) cached)", outcome.source)
        }
        SourceRefreshStatus::Unsupported { reason } => {
            format!("{}: not refreshable ({reason})", outcome.source)
        }
        SourceRefreshStatus::Failed { reason } => format!("{}: failed ({reason})", outcome.source),
        SourceRefreshStatus::Cancelled => format!("{}: cancelled", outcome.source),
    }
}

#[cfg(test)]
#[path = "catalogue_refresh_bridge_tests.rs"]
mod tests;
