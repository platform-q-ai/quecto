//! Provider runtime composition adapter for the agent CLI.

use crate::domain::provider::LlmProvider;
use crate::infrastructure::config::Config;
use std::sync::Arc;

pub fn build_agent_provider(
    config: &Config,
    base_dir: &std::path::Path,
    http_client: &reqwest::Client,
) -> Result<Arc<dyn LlmProvider>, String> {
    use crate::infrastructure::provider_runtime::provider_runtime_application as ProviderRuntimeApplication;
    let application = ProviderRuntimeApplication();
    application.compose(config, &(base_dir, http_client))
}
