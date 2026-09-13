//! Pure provider vocabulary: streaming events, cancellation, thinking and
//! effort levels, tool choice and request metadata. The provider port
//! (`LlmProvider`), the per-request admission port (`RequestAdmission`) and
//! the `ChatRequest` they exchange are the application's
//! (`application::providers::ports`, #1960).
use super::message::LlmResponse;

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

#[cfg(test)]
#[path = "provider_tests.rs"]
mod tests;
