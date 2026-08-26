//! Thin provider entry point for the agent CLI (epic #1193, slice 3).
//!
//! Provider construction, credential resolution, and router orchestration
//! live in `infrastructure::provider_runtime`; this module only invokes the
//! shared compose-provider-runtime use case via the interface wiring and
//! hands back the routing provider.

use std::sync::Arc;

use crate::domain::provider::LlmProvider;
use crate::infrastructure::config::Config;

/// Compose and publish the provider runtime for this base directory, then
/// return its routing provider. The published runtime and catalogue always
/// describe one coherent generation; a failed composition retains the
/// previously published one.
pub fn build_agent_provider(
    config: &Config,
    base_dir: &std::path::Path,
    http_client: &reqwest::Client,
) -> Result<Arc<dyn LlmProvider>, String> {
    crate::interface::catalogue_runtime::compose_and_publish_runtime(config, base_dir, http_client)
        .map(|snapshot| snapshot.provider.clone())
        .map_err(|error| error.error)
}
