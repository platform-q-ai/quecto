//! Compatibility provider-only entry point for legacy callers.

use std::sync::Arc;

use crate::domain::provider::LlmProvider;
use crate::infrastructure::config::Config;

pub fn build_agent_provider(
    config: &Config,
    base_dir: &std::path::Path,
    http_client: &reqwest::Client,
) -> Result<Arc<dyn LlmProvider>, String> {
    Ok(crate::interface::catalogue_runtime::build_runtime_snapshot(
        config,
        base_dir,
        http_client,
        0,
    )?
    .provider)
}
