//! Interface wiring for application-owned provider runtime composition and
//! model selection (epic #1193, slice 3).
//!
//! The only layer allowed to see both application use cases and infrastructure
//! adapters: entry points (CLI startup, provider reload, UDS model switching)
//! call these functions instead of constructing provider state themselves.

use std::path::Path;
use std::sync::Arc;

use crate::application::provider_runtime::{
    CatalogueRuntimeSnapshot, ComposeProviderRuntimeUseCase, CompositionPorts, ModelSelection,
    ResolveModelSelectionUseCase, RuntimeCompositionError, SelectionError,
};
use crate::domain::catalogue::ModelRef;
use crate::infrastructure::catalogue_inputs::CatalogueInputs;
use crate::infrastructure::catalogue_registry::{runtime_store_for, snapshot_store_for};
use crate::infrastructure::config::Config;
use crate::infrastructure::provider_runtime::{AgentProviderRuntimeFactory, AgentRuntimeInputs};

/// Compose the concrete provider runtime for `base_dir` via the shared use
/// case and publish runtime + catalogue as one coherent generation into the
/// process-wide stores for that directory. A failed composition retains the
/// previously published generation (echoed in the error).
pub fn compose_and_publish_runtime(
    config: &Config,
    base_dir: &Path,
    http_client: &reqwest::Client,
) -> Result<Arc<CatalogueRuntimeSnapshot>, RuntimeCompositionError> {
    let catalogue_inputs = CatalogueInputs::load(base_dir);
    let inputs = AgentRuntimeInputs {
        base_dir: base_dir.to_path_buf(),
        http_client: http_client.clone(),
        refresh_fn: crate::interface::shared::make_oauth_refresh_fn(),
        openai_oauth_factory: crate::interface::shared::make_provider_factory(
            "openai",
            openai_api_base(config),
            http_client.clone(),
        ),
        // Same on-disk read as the catalogue sources above: one compose never
        // pairs a catalogue and a router built from different models.json
        // states (and models.json is parsed once per compose, not twice).
        model_registry: catalogue_inputs.effective_registry(),
    };
    let catalogue_store = snapshot_store_for(base_dir);
    let runtime_store = runtime_store_for(base_dir);
    let composed = ComposeProviderRuntimeUseCase::new().compose_and_publish(
        &AgentProviderRuntimeFactory,
        config,
        &inputs,
        &CompositionPorts {
            sources: &catalogue_inputs.sources(),
            credentials: &catalogue_inputs.credentials,
            catalogue_store: &catalogue_store,
            runtime_store: &runtime_store,
        },
    )?;
    Ok(composed.snapshot)
}

/// Delegates to the factory's own blank/trim helper so the initially composed
/// provider and the post-refresh rebuilt provider share one base-URL reading.
fn openai_api_base(config: &Config) -> Option<String> {
    crate::infrastructure::provider_runtime::non_empty(config.providers.openai.api_base.clone())
}

/// Resolve a qualified model reference against the published runtime
/// generation for `base_dir`: the catalogue identity plus runnable provider,
/// or the structured reason it cannot run. An unparsable reference maps to
/// the unknown-model reason (the catalogue can never know it).
pub fn select_model(base_dir: &Path, qualified: &str) -> Result<ModelSelection, SelectionError> {
    let Ok(reference) = ModelRef::parse_qualified(qualified) else {
        return Err(SelectionError::UnknownModel {
            reference: qualified.to_string(),
        });
    };
    ResolveModelSelectionUseCase::new().select(&runtime_store_for(base_dir), &reference)
}

use crate::application::catalogue::{ResolveCatalogueUseCase, ResolvedCatalogue, model_limits_in};
use crate::application::catalogue_refresh::{
    CatalogueRefreshReport, RefreshBounds, RefreshCatalogueSourcesUseCase, RefreshContext,
    RefreshPorts, RefreshSelection, SourceRefreshOutcome, SourceRefreshStatus,
};
use crate::application::ports::CatalogueSnapshotStore;
use crate::infrastructure::catalogue_discovery::{SecretsRedaction, configured_discovery};

/// Run the resolve-effective-catalogue use case over the real sources for
/// `base_dir` and publish into its shared store. Startup calls this once to
/// publish the initial generation; the read surfaces call it again to stay
/// level with on-disk edits until explicit refresh arrives (epic #1193 slices
/// 4-5). No network is touched.
pub fn resolve_and_publish_for(base_dir: &Path) -> (CatalogueSnapshotStore, ResolvedCatalogue) {
    let store = snapshot_store_for(base_dir);
    let inputs = CatalogueInputs::load(base_dir);
    let resolved =
        ResolveCatalogueUseCase.resolve_and_publish(&inputs.sources(), &inputs.credentials, &store);
    (store, resolved)
}

/// The per-model limits for a qualified `provider/model` string, read from
/// the published catalogue snapshot: `(output cap, context window)`, each
/// `None` unless explicitly declared. One resolve-and-publish feeds one
/// `model_limits_in` read, so the limits a consumer applies always describe
/// the generation it just published — no surface re-derives limits from the
/// registry on its own (epic #1193, slice 6).
pub fn published_model_limits(base_dir: &Path, qualified: &str) -> (Option<u32>, Option<usize>) {
    let (store, _) = resolve_and_publish_for(base_dir);
    model_limits_in(&store.current(), qualified)
}

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
#[path = "catalogue_runtime_refresh_tests.rs"]
mod refresh_tests;
#[cfg(test)]
#[path = "catalogue_runtime_resolve_tests.rs"]
mod resolve_tests;
#[cfg(test)]
#[path = "catalogue_runtime_tests.rs"]
mod tests;
