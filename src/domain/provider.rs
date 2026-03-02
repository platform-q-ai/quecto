use std::future::Future;
use std::pin::Pin;

use super::{
    error::DomainError,
    message::{LlmResponse, Message},
    tool::ToolDefinition,
};

/// Parameters for a chat request to an LLM provider.
#[derive(Debug, Clone)]
pub struct ChatRequest<'a> {
    pub messages: &'a [Message],
    pub tools: &'a [ToolDefinition],
    pub model: &'a str,
    pub max_tokens: u32,
    pub temperature: f32,
    /// Optional session identifier for providers that support prompt caching
    /// keyed by session (e.g. Codex `prompt_cache_key`).
    pub session_id: Option<String>,
}

/// Determine whether a model name is definitively owned by a named provider family,
/// meaning no other provider should attempt to serve it.
///
/// Rules (vendor-assigned prefixes, not user-configurable):
/// - `claude-*` (case-insensitive) → owned by `"anthropic"`
///
/// Returns `true` when `model` has a known owner that is NOT `provider_name`,
/// i.e. `provider_name` should be skipped for this model.
///
/// Unknown model names return `false` (any provider may attempt them).
pub fn model_excluded_from_provider(model: &str, provider_name: &str) -> bool {
    // Use a byte-level ASCII prefix check — zero allocation, O(n) in prefix length.
    if model.len() >= 7 && model[..7].eq_ignore_ascii_case("claude-") {
        return provider_name != "anthropic";
    }
    false
}

/// Port: an LLM provider that can process chat requests.
pub trait LlmProvider: Send + Sync + std::fmt::Debug {
    /// Human-readable provider name (e.g. "openai", "anthropic").
    fn name(&self) -> &str;

    /// Send a chat request and return the LLM response.
    fn chat(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>>;

    /// Send a streaming chat request and return the assembled response.
    ///
    /// Default implementation delegates to `chat()` (non-streaming).
    /// Providers that support SSE streaming override this method.
    fn chat_stream(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        self.chat(request)
    }
}

#[cfg(test)]
mod routing_tests {
    use super::*;

    #[test]
    fn claude_model_excluded_from_openai() {
        assert!(model_excluded_from_provider("claude-opus-4-5", "openai"));
        assert!(model_excluded_from_provider(
            "claude-sonnet-4-20250514",
            "openai"
        ));
        assert!(model_excluded_from_provider("claude-3-5-sonnet", "openai"));
    }

    #[test]
    fn claude_model_not_excluded_from_anthropic() {
        assert!(!model_excluded_from_provider(
            "claude-opus-4-5",
            "anthropic"
        ));
        assert!(!model_excluded_from_provider(
            "claude-sonnet-4-20250514",
            "anthropic"
        ));
    }

    #[test]
    fn claude_prefix_check_is_case_insensitive() {
        // Model names are lowercase in practice but the check must be robust.
        assert!(model_excluded_from_provider("Claude-opus-4-5", "openai"));
        assert!(model_excluded_from_provider("CLAUDE-3-haiku", "openai"));
        assert!(!model_excluded_from_provider("CLAUDE-3-haiku", "anthropic"));
    }

    #[test]
    fn gpt_model_not_excluded_from_any_provider() {
        assert!(!model_excluded_from_provider("gpt-4o", "openai"));
        assert!(!model_excluded_from_provider("gpt-4o", "anthropic"));
    }

    #[test]
    fn unknown_model_not_excluded_from_any_provider() {
        assert!(!model_excluded_from_provider(
            "some-unknown-model",
            "openai"
        ));
        assert!(!model_excluded_from_provider(
            "some-unknown-model",
            "anthropic"
        ));
        assert!(!model_excluded_from_provider("", "openai"));
    }

    #[test]
    fn short_model_name_does_not_panic() {
        // "claude" without trailing dash is shorter than 7 bytes — must not panic
        assert!(!model_excluded_from_provider("claude", "openai"));
        assert!(!model_excluded_from_provider("clau", "openai"));
        assert!(!model_excluded_from_provider("c", "openai"));
    }
}
