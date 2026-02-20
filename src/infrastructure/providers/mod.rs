pub mod anthropic;
pub mod error;
pub mod fallback;
pub mod openai;

use std::sync::Arc;

use crate::domain::provider::LlmProvider;

/// Create a provider by name and API key.
pub fn create_provider(
    name: &str,
    api_key: String,
    api_base: Option<String>,
) -> Option<Arc<dyn LlmProvider>> {
    match name {
        "openai" => Some(Arc::new(openai::OpenAiProvider::new(api_key, api_base))),
        "anthropic" => Some(Arc::new(anthropic::AnthropicProvider::new(
            api_key, api_base,
        ))),
        _ => None,
    }
}
