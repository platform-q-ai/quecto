use std::future::Future;
use std::pin::Pin;

use super::{
    error::DomainError,
    message::{LlmResponse, Message},
    tool::ToolDefinition,
};

/// Incremental streaming event emitted by `chat_stream_incremental()`.
///
/// Callers receive these events as each SSE packet arrives from the LLM,
/// enabling real-time token rendering without buffering the full response.
#[derive(Debug)]
pub enum StreamEvent {
    /// A text token arrived from the LLM.
    TextDelta(String),
    /// An extended-thinking token arrived (supported by select models).
    ThinkingDelta(String),
    /// A tool call started; the model is about to stream its arguments.
    ToolCallStart { id: String, name: String },
    /// A partial JSON fragment of tool call arguments arrived.
    ToolCallDelta(String),
    /// A tool call finished; `arguments` is the fully assembled JSON string.
    ToolCallEnd {
        id: String,
        name: String,
        arguments: String,
    },
    /// The LLM turn is complete. Contains the fully assembled `LlmResponse`.
    Done(LlmResponse),
    /// A terminal error occurred during streaming (human-readable message).
    Error(String),
}

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
    /// Optional tool_choice parameter to control how the model selects tools.
    pub tool_choice: Option<ToolChoice>,
    /// Optional metadata (e.g. user_id for multi-tenant rate limiting).
    pub metadata: Option<RequestMetadata>,
    /// Optional thinking level for extended thinking support.
    /// When set, the Anthropic provider adds a `thinking` parameter to the request.
    pub thinking_level: Option<ThinkingLevel>,
    /// Optional cancellation flag. When `Some`, the provider checks this flag
    /// before processing and returns `DomainError::Provider("request cancelled")`
    /// immediately if it is set. Set `cancel_flag.cancel()` from any thread to
    /// cancel an in-flight request.
    pub cancel_flag: Option<CancelFlag>,
}

/// A shared cancellation flag that can be checked by providers.
///
/// Wraps `Arc<AtomicBool>` as a domain-level concept so that the domain layer
/// does not expose raw concurrency primitives in its public API.
#[derive(Debug, Clone, Default)]
pub struct CancelFlag(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl CancelFlag {
    /// Create a new, unset cancel flag.
    pub fn new() -> Self {
        Self(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
            false,
        )))
    }

    /// Signal cancellation. The next provider check will return a cancellation error.
    pub fn cancel(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Release);
    }

    /// Returns `true` if cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Acquire)
    }
}

/// Effort levels for extended thinking.
///
/// Maps to Anthropic's thinking budget tokens:
/// - `Low` → 1024 tokens
/// - `Medium` → 10000 tokens
/// - `High` → 16384 tokens
/// - `Max` → 32768 tokens
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingLevel {
    Low,
    Medium,
    High,
    Max,
}

impl ThinkingLevel {
    /// Return the thinking budget in tokens for this level.
    pub fn budget_tokens(self) -> u32 {
        match self {
            Self::Low => 1024,
            Self::Medium => 10_000,
            Self::High => 16_384,
            Self::Max => 32_768,
        }
    }
}

/// Controls how the model selects which tool to call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolChoice {
    /// Model decides freely whether to call a tool (default).
    Auto,
    /// Model must call some tool.
    Any,
    /// Model must call the specified tool.
    Specific(String),
}

/// Request metadata for provider-side tracking (e.g. per-user rate limiting).
#[derive(Debug, Clone)]
pub struct RequestMetadata {
    /// User identifier for per-user rate limiting (Anthropic `metadata.user_id`).
    pub user_id: Option<String>,
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

    /// Send a streaming chat request and return a channel that emits
    /// incremental [`StreamEvent`]s as each SSE packet arrives.
    ///
    /// The channel is closed after either a [`StreamEvent::Done`] or
    /// [`StreamEvent::Error`] is sent. Callers should read until the channel
    /// closes or until they receive one of those terminal events.
    ///
    /// Default implementation wraps `chat_stream()` and emits a single
    /// `Done` or `Error` event — no true incremental delivery.
    /// Providers that support byte-stream SSE should override this method.
    fn chat_stream_incremental(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = tokio::sync::mpsc::Receiver<StreamEvent>> + Send + '_>> {
        let fut = self.chat_stream(request);
        Box::pin(async move {
            let (tx, rx) = tokio::sync::mpsc::channel(32);
            match fut.await {
                Ok(resp) => {
                    let _ = tx.send(StreamEvent::Done(resp)).await;
                }
                Err(e) => {
                    let _ = tx.send(StreamEvent::Error(e.to_string())).await;
                }
            }
            rx
        })
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
