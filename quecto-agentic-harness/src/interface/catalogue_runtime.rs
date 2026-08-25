//! Composition-root adapter that builds one application-owned runtime snapshot.

use crate::application::provider_runtime::{CatalogueRuntimeSnapshot, compose_catalogue_runtime};
use crate::infrastructure::config::Config;
use crate::infrastructure::provider_runtime::{
    InfrastructureProviderRuntimeFactory, ProviderRuntimeInputs,
};

pub fn build_runtime_snapshot(
    config: &Config,
    base_dir: &std::path::Path,
    http_client: &reqwest::Client,
    generation: u64,
) -> Result<CatalogueRuntimeSnapshot, String> {
    compose_catalogue_runtime(
        &InfrastructureProviderRuntimeFactory,
        config,
        &ProviderRuntimeInputs {
            base_dir,
            http_client,
        },
        generation,
    )
}

#[cfg(test)]
#[path = "catalogue_runtime_tests.rs"]
mod tests;
