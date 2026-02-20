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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_openai_provider() {
        let provider = create_provider("openai", "sk-test".to_string(), None);
        assert!(provider.is_some());
        assert_eq!(provider.unwrap().name(), "openai");
    }

    #[test]
    fn test_create_anthropic_provider() {
        let provider = create_provider("anthropic", "sk-ant-test".to_string(), None);
        assert!(provider.is_some());
        assert_eq!(provider.unwrap().name(), "anthropic");
    }

    #[test]
    fn test_create_unknown_provider() {
        let provider = create_provider("gemini", "key".to_string(), None);
        assert!(provider.is_none());
    }

    #[test]
    fn test_create_openai_with_custom_base() {
        let provider = create_provider(
            "openai",
            "sk-test".to_string(),
            Some("http://localhost:8080".to_string()),
        );
        assert!(provider.is_some());
    }
}
