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
    /// When `None`, the Anthropic provider defaults to `low` for 4.6 models
    /// (to avoid the API's implicit `high` default); omitted for other models.
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
    ///
    /// Returns `None` for `Adaptive` (which has no fixed budget — use `effort` instead).
    /// Returns `Some(n)` for all manual levels.
    pub fn budget_tokens(self) -> Option<u32> {
        match self {
            Self::Adaptive => None,
            Self::Low => Some(1024),
            Self::Medium => Some(10_000),
            Self::High => Some(16_384),
            Self::Max => Some(32_768),
        }
    }
}

/// Reasoning/output effort level.
///
/// The accepted vocabulary is the union of the providers' documented scales
/// (#1066):
///
/// - OpenAI reasoning models document `none, low, medium, high, xhigh`
///   (transmitted verbatim as `reasoning.effort` on the Responses API).
/// - Anthropic documents `low, medium, high` plus `max` (Opus 4.6 only),
///   emitted as `output_config: {effort: "<level>"}`.
///
/// Each provider adapter maps levels outside its own documented scale to its
/// nearest documented value; parsing rejects anything outside the union.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffortLevel {
    /// OpenAI only: disable reasoning ("none").
    None,
    Low,
    Medium,
    High,
    /// OpenAI only: extra-high reasoning ("xhigh").
    XHigh,
    /// Anthropic only (Opus 4.6): absolute highest capability.
    Max,
}

impl EffortLevel {
    /// Comma-separated list of every accepted effort string, for error messages.
    pub const VALID_VALUES: &'static str = "none, low, medium, high, xhigh, max";

    /// Return the API string value for this effort level.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
        }
    }

    /// OpenAI's documented reasoning-effort scale (#1066).
    pub const OPENAI_LEVELS: &'static [Self] =
        &[Self::None, Self::Low, Self::Medium, Self::High, Self::XHigh];

    /// Anthropic's documented effort scale (`max` is Opus 4.6 only).
    pub const ANTHROPIC_LEVELS: &'static [Self] = &[Self::Low, Self::Medium, Self::High, Self::Max];

    /// The effort vocabulary valid for the provider serving `model`
    /// (a `provider/model-id` pair, or a bare model id).
    ///
    /// Anthropic-served models (provider prefix contains "anthropic", or a
    /// bare `claude-*` id) use [`Self::ANTHROPIC_LEVELS`]; everything else
    /// uses the OpenAI-shaped scale, which is also what OpenAI-compatible
    /// providers accept.
    pub fn levels_for_model(model: &str) -> &'static [Self] {
        let (provider, id) = model.split_once('/').unwrap_or(("", model));
        if provider.contains("anthropic") || id.starts_with("claude") {
            Self::ANTHROPIC_LEVELS
        } else {
            Self::OPENAI_LEVELS
        }
    }

    /// Render a level slice as the comma-separated list used in error
    /// messages and selector vocabularies (e.g. "none, low, medium, high,
    /// xhigh").
    pub fn levels_list(levels: &[Self]) -> String {
        levels
            .iter()
            .map(|l| l.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Parse a string into an `EffortLevel`.
    ///
    /// Returns `None` for unrecognised values.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "none" => Some(Self::None),
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "xhigh" => Some(Self::XHigh),
            "max" => Some(Self::Max),
            _ => None,
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

    /// Downcast support for introspection (e.g. recovering a concrete
    /// `ProviderRouter` for diagnostics and tests). Implementors that need to be
    /// downcast override this to return `self`; the default returns a reference
    /// that downcasts to nothing useful.
    fn as_any(&self) -> &dyn std::any::Any {
        &()
    }

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

#[cfg(test)]
#[path = "provider_cov_tests.rs"]
mod cov_tests;

#[cfg(test)]
mod tests {
    use super::*;

    // --- CancelFlag ---

    #[test]
    fn cancel_flag_initially_not_cancelled() {
        let flag = CancelFlag::new();
        assert!(!flag.is_cancelled());
    }

    #[test]
    fn cancel_flag_cancel_sets_cancelled() {
        let flag = CancelFlag::new();
        flag.cancel();
        assert!(flag.is_cancelled());
    }

    #[test]
    fn cancel_flag_clone_shares_state() {
        let flag = CancelFlag::new();
        let clone = flag.clone();
        flag.cancel();
        assert!(clone.is_cancelled());
    }

    #[test]
    fn cancel_flag_default_not_cancelled() {
        let flag = CancelFlag::default();
        assert!(!flag.is_cancelled());
    }

    // --- ThinkingLevel ---

    #[test]
    fn thinking_level_adaptive_is_adaptive() {
        assert!(ThinkingLevel::Adaptive.is_adaptive());
    }

    #[test]
    fn thinking_level_non_adaptive() {
        assert!(!ThinkingLevel::Low.is_adaptive());
        assert!(!ThinkingLevel::Medium.is_adaptive());
        assert!(!ThinkingLevel::High.is_adaptive());
        assert!(!ThinkingLevel::Max.is_adaptive());
    }

    #[test]
    fn thinking_level_budget_tokens() {
        assert_eq!(ThinkingLevel::Adaptive.budget_tokens(), None);
        assert_eq!(ThinkingLevel::Low.budget_tokens(), Some(1024));
        assert_eq!(ThinkingLevel::Medium.budget_tokens(), Some(10_000));
        assert_eq!(ThinkingLevel::High.budget_tokens(), Some(16_384));
        assert_eq!(ThinkingLevel::Max.budget_tokens(), Some(32_768));
    }

    // --- EffortLevel ---

    #[test]
    fn effort_level_as_str() {
        assert_eq!(EffortLevel::Low.as_str(), "low");
        assert_eq!(EffortLevel::Medium.as_str(), "medium");
        assert_eq!(EffortLevel::High.as_str(), "high");
        assert_eq!(EffortLevel::Max.as_str(), "max");
    }

    #[test]
    fn effort_level_parse_valid() {
        assert_eq!(EffortLevel::parse("low"), Some(EffortLevel::Low));
        assert_eq!(EffortLevel::parse("medium"), Some(EffortLevel::Medium));
        assert_eq!(EffortLevel::parse("high"), Some(EffortLevel::High));
        assert_eq!(EffortLevel::parse("max"), Some(EffortLevel::Max));
    }

    /// Issue #1066: OpenAI's documented reasoning-effort scale (none, low,
    /// medium, high, xhigh) must be parseable and round-trip through as_str
    /// so it can be transmitted verbatim for OpenAI reasoning models.
    #[test]
    fn effort_level_parse_openai_documented_scale_1066() {
        assert_eq!(EffortLevel::None.as_str(), "none");
        assert_eq!(EffortLevel::XHigh.as_str(), "xhigh");
        for level in ["none", "low", "medium", "high", "xhigh"] {
            let parsed = EffortLevel::parse(level).unwrap_or_else(|| {
                panic!("OpenAI-documented effort level '{level}' must parse (#1066)")
            });
            assert_eq!(
                parsed.as_str(),
                level,
                "effort '{level}' must round-trip verbatim (#1066)"
            );
        }
    }

    #[test]
    fn effort_level_parse_invalid() {
        assert_eq!(EffortLevel::parse(""), None);
        assert_eq!(EffortLevel::parse("ultra"), None);
        assert_eq!(EffortLevel::parse("LOW"), None);
    }

    // --- ToolChoice ---

    #[test]
    fn tool_choice_auto_eq() {
        assert_eq!(ToolChoice::Auto, ToolChoice::Auto);
        assert_ne!(ToolChoice::Auto, ToolChoice::Any);
    }

    #[test]
    fn tool_choice_specific() {
        let tc = ToolChoice::Specific("bash".to_string());
        assert_eq!(tc, ToolChoice::Specific("bash".to_string()));
        assert_ne!(tc, ToolChoice::Specific("read".to_string()));
    }
}

#[cfg(test)]
mod default_surface_cov_tests {
    use super::*;

    #[derive(Debug)]
    struct DefaultProvider;

    impl LlmProvider for DefaultProvider {
        fn name(&self) -> &str {
            "default-provider"
        }
        fn chat<'a>(
            &'a self,
            _request: ChatRequest<'a>,
        ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
            Box::pin(async {
                Ok(LlmResponse {
                    content: Some("ok".into()),
                    tool_calls: vec![],
                    usage: None,
                    stop_reason: None,
                    thinking_blocks: vec![],
                })
            })
        }
    }

    fn req<'a>() -> ChatRequest<'a> {
        ChatRequest {
            messages: &[],
            tools: &[],
            model: "m",
            max_tokens: 1,
            temperature: 0.0,
            session_id: None,
            tool_choice: None,
            metadata: None,
            thinking_level: None,
            cancel_flag: None,
            effort: None,
        }
    }

    #[tokio::test]
    async fn default_provider_methods_execute_for_concrete_impl() {
        let provider = DefaultProvider;
        assert_eq!(provider.name(), "default-provider");
        assert!(provider.as_any().downcast_ref::<()>().is_some());
        assert_eq!(
            provider
                .chat_stream(req())
                .await
                .unwrap()
                .content
                .as_deref(),
            Some("ok")
        );
        let mut rx = provider.chat_stream_incremental(req()).await;
        match rx.recv().await.unwrap() {
            StreamEvent::Done(resp) => assert_eq!(resp.content.as_deref(), Some("ok")),
            other => panic!("unexpected stream event: {other:?}"),
        }
    }
}
