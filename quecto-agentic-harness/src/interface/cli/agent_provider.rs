//! Provider runtime composition adapter for the agent CLI.

use std::sync::Arc;

use crate::application::provider_runtime::{CatalogueRuntimeSnapshot, compose_catalogue_runtime};
use crate::domain::provider::LlmProvider;
use crate::infrastructure::config::Config;
use crate::infrastructure::provider_runtime::{
    InfrastructureProviderRuntimeFactory, ProviderRuntimeInputs,
};

/// Build provider routing and catalogue through the shared application path.
pub fn build_agent_runtime(
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

/// Compatibility helper for call sites that need only provider execution.
pub fn build_agent_provider(
    config: &Config,
    base_dir: &std::path::Path,
    http_client: &reqwest::Client,
) -> Result<Arc<dyn LlmProvider>, String> {
    Ok(build_agent_runtime(config, base_dir, http_client, 0)?.provider)
}
