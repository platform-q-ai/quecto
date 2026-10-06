//! Pure provider vocabulary: streaming events, cancellation, thinking and
//! effort levels, tool choice and request metadata. The provider port
//! (`LlmProvider`), the per-request admission port (`RequestAdmission`) and
//! the `ChatRequest` they exchange are the application's
//! (`application::providers::ports`, #1960).
use crate::domain::message::LlmResponse;

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

#[path = "../../cancel_flag.rs"]
mod cancel_flag;
pub use cancel_flag::{CancelFlag, CancelWatch};

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
/// (#1066): OpenAI's `none, low, medium, high, xhigh` and Anthropic's
/// `low, medium, high, max`. Which subset a given model accepts is the
/// catalogue's per-model capability (`domain::catalogue::EffortVocabulary`,
/// #1996); parsing here is syntax only and rejects anything outside the union.
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

/// Which send of one logical model request an admission check gates
/// (#2339): the agent loop admits the request's [`First`](Self::First)
/// send once, and every later send (a transient-failure retry, a stream
/// re-initiation, a resend after an OAuth refresh) is a
/// [`Reattempt`](Self::Reattempt), re-checked because the run may have
/// been paused, stopped or run out of budget while the failed send and
/// its backoff took their time. The admission records the two apart, so
/// the event log counts one first check per request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestAttempt {
    First,
    Reattempt,
}

/// Where a request for a model goes (#2421 review L1): the one rule the
/// provider router sends by and the catalogue reads a model's entry by, so
/// what the catalogue says a model takes is what the request reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelRoute<'p, 'm> {
    /// To `provider`, which is sent the bare `model` id.
    To { provider: &'p str, model: &'m str },
    /// A `provider/model` id whose provider is none of those configured.
    UnknownProvider { prefix: &'m str },
    /// No provider is configured.
    NoProviders,
}

/// Where a request for `model` goes among `providers`, in routing order: a
/// `provider/model` id to the provider its prefix names (whatever its
/// case), a bare id to the first provider, whether or not that provider
/// lists the model. The split is at the first slash; the model id is
/// opaque and may hold slashes of its own.
pub fn route_model<'p, 'm>(model: &'m str, providers: &[&'p str]) -> ModelRoute<'p, 'm> {
    match parse_qualified_model(model) {
        Some((prefix, id)) => providers
            .iter()
            .find(|name| provider_prefix_matches(prefix, name))
            .map_or(ModelRoute::UnknownProvider { prefix }, |provider| {
                ModelRoute::To {
                    provider,
                    model: id,
                }
            }),
        None => providers
            .first()
            .map_or(ModelRoute::NoProviders, |provider| ModelRoute::To {
                provider,
                model,
            }),
    }
}

/// A `provider/model-id` string as its provider and model id, each trimmed;
/// `None` for a bare id (no slash) or an empty side. The split is at the
/// first slash: the model id is opaque and may hold slashes of its own
/// (Fireworks' `accounts/fireworks/models/glm-5p2`).
pub fn parse_qualified_model(model: &str) -> Option<(&str, &str)> {
    model
        .split_once('/')
        .map(|(prefix, id)| (prefix.trim(), id.trim()))
        .filter(|(prefix, id)| !prefix.is_empty() && !id.is_empty())
}

/// Whether `prefix` names the provider called `provider_name`: the same
/// name in any case, or the historical `openai-codex` alias of `codex`.
/// OAuth/API billing modes are explicit (`openai-api`, `openai-oauth`,
/// `anthropic-api`, `anthropic-oauth`); a bare vendor prefix is aliased to
/// neither, which could silently select the other billing mode.
pub fn provider_prefix_matches(prefix: &str, provider_name: &str) -> bool {
    prefix.eq_ignore_ascii_case(provider_name)
        || (prefix.eq_ignore_ascii_case("openai-codex")
            && provider_name.eq_ignore_ascii_case("codex"))
}

#[cfg(test)]
#[path = "provider_tests.rs"]
mod tests;
