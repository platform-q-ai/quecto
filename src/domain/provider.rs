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
    pub session_id: Option<&'a str>,
    /// Optional tool_choice parameter to control how the model selects tools.
    pub tool_choice: Option<ToolChoice>,
    /// Optional metadata (e.g. user_id for multi-tenant rate limiting).
    pub metadata: Option<RequestMetadata>,
    /// Optional thinking level for extended thinking support.
    /// When set, the Anthropic provider adds a `thinking` parameter to the request.
    /// Use `ThinkingLevel::Adaptive` for Opus 4.6 / Sonnet 4.6 (recommended).
    /// Use `ThinkingLevel::Low/Medium/High/Max` for older models (manual budget).
    pub thinking_level: Option<ThinkingLevel>,
    /// Optional cancellation flag. When `Some`, the provider checks this flag
    /// before processing and returns `DomainError::Provider("request cancelled")`
    /// immediately if it is set. Set `cancel_flag.cancel()` from any thread to
    /// cancel an in-flight request.
    pub cancel_flag: Option<CancelFlag>,
    /// Optional effort level for the `output_config.effort` API parameter.
    /// Controls thinking depth and token spend on Opus 4.6 / Sonnet 4.6.
    /// When `None`, the field is omitted (API defaults to `high`).
    pub effort: Option<EffortLevel>,
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

/// Thinking mode for extended thinking support.
///
/// - `Adaptive` — Opus 4.6 / Sonnet 4.6 recommended mode. Claude dynamically
///   decides when and how much to think. Use with `EffortLevel` to guide depth.
///   Emits `thinking: {type: "adaptive"}` in the API request. No `budget_tokens`.
/// - `Low` / `Medium` / `High` / `Max` — Manual budget mode for older models
///   (Opus 4.5, Sonnet 4.5, etc.). Emits `thinking: {type: "enabled", budget_tokens: N}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingLevel {
    /// Adaptive thinking (Opus 4.6 / Sonnet 4.6). Claude decides the budget.
    Adaptive,
    Low,
    Medium,
    High,
    Max,
}

impl ThinkingLevel {
    /// Return `true` if this level uses adaptive mode (no fixed budget_tokens).
    pub fn is_adaptive(self) -> bool {
        matches!(self, Self::Adaptive)
    }

    /// Return the thinking budget in tokens for manual-mode levels.
    /// Panics if called on `Adaptive` (which has no fixed budget).
    pub fn budget_tokens(self) -> u32 {
        match self {
            Self::Adaptive => panic!("Adaptive thinking has no fixed budget_tokens"),
            Self::Low => 1024,
            Self::Medium => 10_000,
            Self::High => 16_384,
            Self::Max => 32_768,
        }
    }
}

/// Effort level for the `output_config.effort` API parameter.
///
/// Controls how eagerly Claude spends tokens. Works with adaptive thinking on
/// Opus 4.6 / Sonnet 4.6, and alongside manual thinking on Opus 4.5.
/// Emitted as `output_config: {effort: "<level>"}` in the request body.
///
/// - `Low` — fastest, fewest tokens, may skip thinking on simple tasks.
/// - `Medium` — balanced speed/quality.
/// - `High` — maximum quality (default when omitted).
/// - `Max` — absolute highest capability; **Opus 4.6 only**.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffortLevel {
    Low,
    Medium,
    High,
    Max,
}

impl EffortLevel {
    /// Return the API string value for this effort level.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Max => "max",
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

/// Port: an LLM provider that can process chat requests.
pub trait LlmProvider: Send + Sync + std::fmt::Debug {
    /// Human-readable provider name (e.g. "openai", "anthropic").
    fn name(&self) -> &str;

    /// Send a chat request and return the LLM response.
    ///
    /// The lifetime `'a` ties `&self`, the request's borrowed data, and
    /// the returned future together. This allows wrappers (e.g. routers)
    /// to forward borrowed slices without cloning.
    fn chat<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>>;

    /// Send a streaming chat request and return the assembled response.
    ///
    /// Default implementation delegates to `chat()` (non-streaming).
    /// Providers that support SSE streaming override this method.
    fn chat_stream<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
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
    fn chat_stream_incremental<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = tokio::sync::mpsc::Receiver<StreamEvent>> + Send + 'a>> {
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
