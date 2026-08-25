//! Composition-root adapter that builds one application-owned runtime snapshot.

use crate::application::provider_runtime::{CatalogueRuntimeSnapshot, compose_catalogue_runtime};
use crate::infrastructure::catalogue_registry::{
    BuiltinCatalogueSource, UserModelsJsonCatalogueSource,
};
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
    // Precedence, lowest first: built-in metadata, user-owned `models.json`,
    // then the composed runtime layer added by the application use case.
    let builtin = BuiltinCatalogueSource;
    let user = UserModelsJsonCatalogueSource::from_base_dir(base_dir);
    compose_catalogue_runtime(
        &InfrastructureProviderRuntimeFactory,
        config,
        &ProviderRuntimeInputs {
            base_dir,
            http_client,
        },
        generation,
        &[&builtin, &user],
        open_endpoint_providers(config),
    )
}

/// Explicitly configured OpenAI-compatible endpoints route any model id under
/// their prefix, so the catalogue records the prefix rather than pretending to
/// enumerate that provider's models.
fn open_endpoint_providers(config: &Config) -> Vec<crate::domain::catalogue::ProviderId> {
    config
        .providers
        .openai_compatible
        .endpoints
        .iter()
        .filter(|endpoint| !endpoint.api_key.trim().is_empty())
        .filter_map(|endpoint| {
            crate::domain::catalogue::ProviderId::new(endpoint.prefix.trim().to_string()).ok()
        })
        .collect()
}

#[cfg(test)]
#[path = "catalogue_runtime_tests.rs"]
mod tests;
