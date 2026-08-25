//! Provider runtime composition compatibility adapter for the agent CLI.
//!
//! Concrete provider construction lives in infrastructure. This module intentionally
//! stays thin so CLI remains an interface adapter.

use std::sync::Arc;

use crate::domain::provider::LlmProvider;
use crate::infrastructure::config::Config;
use crate::infrastructure::provider_runtime::build_agent_provider as build_runtime;

/// Build the agent provider runtime through infrastructure composition.
pub fn build_agent_provider(
    config: &Config,
    base_dir: &std::path::Path,
    http_client: &reqwest::Client,
) -> Result<Arc<dyn LlmProvider>, String> {
    build_runtime(config, base_dir, http_client)
}
