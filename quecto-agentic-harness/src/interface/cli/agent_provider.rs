//! Provider runtime composition compatibility adapter for the agent CLI.
//!
//! Concrete provider construction lives in infrastructure behind the
//! application-owned [`ProviderRuntimeFactory`] port. This module intentionally
//! stays thin so CLI remains an interface adapter.

use std::sync::Arc;

use crate::domain::provider::LlmProvider;
use crate::infrastructure::config::Config;
use crate::infrastructure::provider_runtime::InfrastructureProviderRuntimeFactory;

/// Build the agent provider runtime through the application composition use case.
pub fn build_agent_provider(
    config: &Config,
    base_dir: &std::path::Path,
    http_client: &reqwest::Client,
) -> Result<Arc<dyn LlmProvider>, String> {
    crate::application::provider_runtime::ComposeProviderRuntimeUseCase::new().compose(
        &InfrastructureProviderRuntimeFactory,
        config,
        base_dir,
        http_client,
    )
}
