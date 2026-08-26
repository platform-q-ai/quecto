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
use crate::infrastructure::catalogue_registry::{runtime_store_for, snapshot_store_for};
use crate::infrastructure::config::Config;
use crate::infrastructure::provider_runtime::{AgentProviderRuntimeFactory, AgentRuntimeInputs};
use crate::interface::cli::catalogue_bridge::CatalogueInputs;

/// Compose the concrete provider runtime for `base_dir` via the shared use
/// case and publish runtime + catalogue as one coherent generation into the
/// process-wide stores for that directory. A failed composition retains the
/// previously published generation (echoed in the error).
pub fn compose_and_publish_runtime(
    config: &Config,
    base_dir: &Path,
    http_client: &reqwest::Client,
) -> Result<Arc<CatalogueRuntimeSnapshot>, RuntimeCompositionError> {
    let inputs = AgentRuntimeInputs {
        base_dir: base_dir.to_path_buf(),
        http_client: http_client.clone(),
        refresh_fn: crate::interface::shared::make_oauth_refresh_fn(),
        openai_oauth_factory: crate::interface::shared::make_provider_factory(
            "openai",
            openai_api_base(config),
            http_client.clone(),
        ),
    };
    let catalogue_inputs = CatalogueInputs::load(base_dir);
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

fn openai_api_base(config: &Config) -> Option<String> {
    let base = config.providers.openai.api_base.trim();
    if base.is_empty() {
        None
    } else {
        Some(base.to_string())
    }
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

#[cfg(test)]
#[path = "catalogue_runtime_tests.rs"]
mod tests;
