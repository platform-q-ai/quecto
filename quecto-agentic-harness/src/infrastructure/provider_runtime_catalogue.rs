use std::sync::Arc;

use crate::domain::catalogue::ModelDescriptor;
use crate::domain::provider::LlmProvider;
use crate::infrastructure::config::Config;
use crate::infrastructure::provider_runtime::build_agent_provider;

pub(crate) fn build_agent_provider_with_descriptors(
    config: &Config,
    base_dir: &std::path::Path,
    http_client: &reqwest::Client,
) -> Result<(Arc<dyn LlmProvider>, Vec<ModelDescriptor>), String> {
    let provider = build_agent_provider(config, base_dir, http_client)?;
    let descriptors = provider.model_descriptors().unwrap_or(&[]).to_vec();
    Ok((provider, descriptors))
}
