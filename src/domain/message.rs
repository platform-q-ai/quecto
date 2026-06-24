/// A single message in a conversation.
#[derive(Debug, Clone, Default)]
pub struct Message {
    pub role: Role,
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    /// When role is Tool, this holds the tool_call id being responded to.
    pub tool_call_id: Option<String>,
    /// Agent-loop turn number when this message was appended.
    pub turn: Option<u32>,
    /// Whether this message is pinned (never dropped by sliding window).
    pub is_pinned: bool,
    /// Whether this message is the spill manifest.
    pub is_manifest: bool,
    /// Whether this tool result has already been collapsed.
    pub is_collapsed: bool,
    /// Tool name for tool result messages.
    pub tool_name: Option<String>,
    /// First chars of tool input (for collapse preview).
    pub input_preview: Option<String>,
    /// Spill ID for recall() lookup.
    pub spill_id: Option<String>,
    /// Image blocks for tool result messages that return image data (e.g. `read` on images).
    /// Empty for non-image messages. Not sent to context-pruning; passed directly to providers.
    pub image_blocks: Vec<crate::domain::tool::ImageBlock>,
    /// Whether this tool result represents an error (propagated to Anthropic `is_error` field).
    pub is_error: bool,
    /// Stop reason from the LLM for assistant messages.
    ///
    /// Used by the normalization pipeline to filter out incomplete assistant
    /// messages (e.g. those that ended with an error) before sending to the API.
    pub stop_reason: Option<StopReason>,
    /// Inline image blocks attached to a **user** message.
    ///
    /// Distinct from `image_blocks` (which is for tool results). When non-empty,
    /// the provider builds a structured content block array instead of a plain string.
    ///
    /// **Transient** — intentionally not persisted in `FileSessionStore`.
    /// Session files store only the text portion of user messages; if a session
    /// is reloaded the image content will not be replayed. This matches the
    /// expected usage pattern (images are sent once in the active session).
    pub user_image_blocks: Vec<UserImageBlock>,
    /// Extended thinking blocks from assistant messages.
    ///
    /// Anthropic's thinking-capable models (Sonnet 4.5+, Opus 4.5+) emit
    /// `thinking` and `redacted_thinking` content blocks alongside text and
    /// tool_use blocks. These must be replayed verbatim (with their cryptographic
    /// signatures) in multi-turn conversations.
    ///
    /// Stored as a `Vec` because a single assistant turn can interleave multiple
    /// thinking blocks with text/tool_use blocks.
    pub thinking_blocks: Vec<ThinkingBlock>,
}

/// A thinking content block from an assistant message.
///
/// Anthropic's extended thinking produces two block types:
/// - **Normal**: Contains the reasoning text and a cryptographic signature.
///   The signature is required for replaying the block in subsequent turns.
/// - **Redacted**: Contains only an opaque `data` payload (reasoning hidden).
///   Must be passed back verbatim as `redacted_thinking` in subsequent turns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThinkingBlock {
    /// Normal thinking block with visible reasoning text and signature.
    Normal {
        /// The model's chain-of-thought reasoning text.
        thinking: String,
        /// Cryptographic signature for the thinking block.
        /// Required by Anthropic's API for replaying thinking in multi-turn.
        signature: String,
    },
    /// Redacted thinking block (reasoning hidden by safety filters).
    Redacted {
        /// Opaque encrypted payload — must be passed back verbatim.
        data: String,
    },
}

/// An image block attached directly to a user message.
///
/// Unlike `crate::domain::tool::ImageBlock` (which uses `&'static str` for
/// `mime_type`), this variant owns its strings to support runtime MIME types
/// from user-provided files.
#[derive(Debug, Clone)]
pub struct UserImageBlock {
    /// MIME type string, e.g. `"image/png"`, `"image/jpeg"`.
    pub mime_type: String,
    /// Base64-encoded image bytes (standard encoding, no line breaks).
    pub data: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            is_pinned: true,
            ..Default::default()
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            ..Default::default()
        }
    }

    pub fn assistant(content: impl Into<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_calls,
            ..Default::default()
        }
    }

    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_call_id: Some(tool_call_id.into()),
            ..Default::default()
        }
    }
}

/// The role of a message sender.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Role {
    System,
    #[default]
    User,
    Assistant,
    Tool,
}

/// A tool invocation requested by the LLM.
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// JSON-encoded arguments string.
    pub arguments: String,
}

/// A complete response from an LLM provider.
#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Option<UsageInfo>,
    /// The reason the model stopped generating (e.g. end_turn, max_tokens, tool_use).
    pub stop_reason: Option<StopReason>,
    /// Thinking blocks from the response (for multi-turn replay).
    /// Populated by the Anthropic provider when extended thinking is enabled.
    pub thinking_blocks: Vec<ThinkingBlock>,
}

/// Why the model stopped generating output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    /// Normal end of response.
    EndTurn,
    /// Response was truncated due to max_tokens limit.
    MaxTokens,
    /// Model is requesting tool execution.
    ToolUse,
    /// Model refused the request.
    Refusal,
    /// An error occurred (e.g. safety filter).
    Error,
    /// Request was cancelled by the caller before or during generation.
    Aborted,
    /// Unknown stop reason (future-proofing).
    Unknown(String),
}

impl StopReason {
    /// Parse a stop reason string into a `StopReason` variant.
    ///
    /// Accepts the canonical strings used by Anthropic (`end_turn`,
    /// `max_tokens`, `tool_use`, etc.) which are also used as the
    /// serialisation format in `FileSessionStore`. Provider-specific
    /// aliases (`pause_turn`, `stop_sequence`, `sensitive`) are mapped
    /// to the appropriate canonical variant.
    pub fn parse(reason: &str) -> Self {
        match reason {
            "end_turn" => Self::EndTurn,
            "max_tokens" | "model_context_window_exceeded" => Self::MaxTokens,
            "tool_use" => Self::ToolUse,
            "refusal" => Self::Refusal,
            "pause_turn" | "stop_sequence" => Self::EndTurn,
            "sensitive" | "error" => Self::Error,
            "aborted" => Self::Aborted,
            other => Self::Unknown(other.to_string()),
        }
    }

    /// Return the canonical string representation for this stop reason.
    pub fn as_str(&self) -> &str {
        match self {
            Self::EndTurn => "end_turn",
            Self::MaxTokens => "max_tokens",
            Self::ToolUse => "tool_use",
            Self::Refusal => "refusal",
            Self::Error => "error",
            Self::Aborted => "aborted",
            Self::Unknown(s) => s.as_str(),
        }
    }
}

impl std::fmt::Display for StopReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Token usage information from an LLM call.
#[derive(Debug, Clone)]
pub struct UsageInfo {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    /// Tokens served from prompt cache (Anthropic `cache_read_input_tokens`).
    pub cache_read_tokens: Option<u32>,
    /// Tokens written to prompt cache (Anthropic `cache_creation_input_tokens`).
    pub cache_write_tokens: Option<u32>,
    /// True context-window occupancy for this turn, normalized across providers.
    ///
    /// This exists because providers report prompt size differently when prompt
    /// caching is active:
    ///   - OpenAI/Codex: `prompt_tokens`/`input_tokens` already counts the full
    ///     prompt (cached tokens are a *subset*), so this is left `None` and the
    ///     context gauge falls back to `prompt_tokens`.
    ///   - Anthropic: `input_tokens` counts only the *non-cached* delta; the
    ///     cached portion is reported separately in `cache_read_input_tokens`
    ///     and `cache_creation_input_tokens`. True occupancy is the sum, set
    ///     here so the context gauge does not undercount on warm sessions.
    ///
    /// Billing (`prompt_tokens` + the discounted cache fields, via
    /// [`ModelPricing::cost_for`]) intentionally does *not* use this field.
    pub context_tokens: Option<u32>,
    /// Per-call cost breakdown, if model pricing is available.
    pub cost: Option<CostInfo>,
}

impl UsageInfo {
    /// Tokens occupying the model context window for this turn.
    ///
    /// Providers that report cached prompt tokens outside `prompt_tokens` set
    /// `context_tokens`; providers whose `prompt_tokens` already includes the
    /// full prompt leave it unset and use `prompt_tokens` as the context gauge.
    pub fn context_input_tokens(&self) -> u32 {
        self.context_tokens.unwrap_or(self.prompt_tokens)
    }
}

/// Per-call cost breakdown calculated from token usage and model pricing.
///
/// Costs are stored internally as **micro-USD** (`u64`, i.e. millionths of a US dollar)
/// to avoid floating-point accumulation errors. Use the `*_usd()` helpers for display.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CostInfo {
    /// Input token cost in micro-USD.
    pub input_cost_micro_usd: u64,
    /// Output token cost in micro-USD.
    pub output_cost_micro_usd: u64,
    /// Cache-read input token cost in micro-USD.
    pub cache_read_cost_micro_usd: u64,
    /// Cache-write input token cost in micro-USD.
    pub cache_write_cost_micro_usd: u64,
    /// Total cost in micro-USD (sum of all components).
    pub total_cost_micro_usd: u64,
}

impl CostInfo {
    /// Input cost in USD.
    pub fn input_cost_usd(&self) -> f64 {
        self.input_cost_micro_usd as f64 / 1_000_000.0
    }
    /// Output cost in USD.
    pub fn output_cost_usd(&self) -> f64 {
        self.output_cost_micro_usd as f64 / 1_000_000.0
    }
    /// Cache-read cost in USD.
    pub fn cache_read_cost_usd(&self) -> f64 {
        self.cache_read_cost_micro_usd as f64 / 1_000_000.0
    }
    /// Total cost in USD.
    pub fn total_cost_usd(&self) -> f64 {
        self.total_cost_micro_usd as f64 / 1_000_000.0
    }
}

/// Per-million-token pricing for a model, stored as micro-USD per million tokens.
/// Using integer rates avoids floating-point representation issues at the pricing layer.
#[derive(Debug, Clone, Copy)]
pub struct ModelPricing {
    /// Input token cost in micro-USD per million tokens.
    pub input_micro_usd_per_million: u64,
    /// Output token cost in micro-USD per million tokens.
    pub output_micro_usd_per_million: u64,
    /// Cache-read input token cost in micro-USD per million tokens.
    pub cache_read_micro_usd_per_million: u64,
    /// Cache-write input token cost in micro-USD per million tokens.
    pub cache_write_micro_usd_per_million: u64,
}

impl ModelPricing {
    /// Calculate cost from usage data using integer micro-USD arithmetic.
    pub fn cost_for(&self, usage: &UsageInfo) -> CostInfo {
        // tokens * rate_per_million / 1_000_000 — performed in u64 to stay exact.
        let calc = |tokens: u32, rate: u64| -> u64 { (tokens as u64 * rate) / 1_000_000 };
        let input = calc(usage.prompt_tokens, self.input_micro_usd_per_million);
        let output = calc(usage.completion_tokens, self.output_micro_usd_per_million);
        let cache_read = calc(
            usage.cache_read_tokens.unwrap_or(0),
            self.cache_read_micro_usd_per_million,
        );
        let cache_write = calc(
            usage.cache_write_tokens.unwrap_or(0),
            self.cache_write_micro_usd_per_million,
        );
        CostInfo {
            input_cost_micro_usd: input,
            output_cost_micro_usd: output,
            cache_read_cost_micro_usd: cache_read,
            cache_write_cost_micro_usd: cache_write,
            total_cost_micro_usd: input + output + cache_read + cache_write,
        }
    }
}

/// Returns true if `model` starts with `prefix`, case-insensitively, using byte
/// comparison — no heap allocation.
pub(crate) fn starts_with_ci(model: &str, prefix: &str) -> bool {
    let m = model.as_bytes();
    let p = prefix.as_bytes();
    if m.len() < p.len() {
        return false;
    }
    m[..p.len()]
        .iter()
        .zip(p)
        .all(|(a, b)| a.eq_ignore_ascii_case(b))
}

/// Look up pricing for a known model. Returns `None` for unknown models.
///
/// **Allowlist**: only `claude-sonnet-4`, `claude-opus-4`, and `claude-haiku-4`
/// families are recognised. Any other model string returns `None`, preventing a
/// spoofed model name from silently matching unintended pricing.
///
/// Rates are expressed as micro-USD per million tokens (integer arithmetic, no f64 drift).
/// Cache write = 1.25× base input (5-minute TTL). Cache read = 0.1× base input.
///
/// Sources (March 2026):
///   Opus 4.6 / 4.5: $5 in / $25 out / $6.25 cache-write / $0.50 cache-read per MTok
///   Sonnet 4.6 / 4.5 / 4: $3 in / $15 out / $3.75 cache-write / $0.30 cache-read per MTok
///   Haiku 4.5: $1 in / $5 out / $1.25 cache-write / $0.10 cache-read per MTok
pub fn model_pricing(model: &str) -> Option<ModelPricing> {
    // Checked in order of expected call frequency (Opus 4.6 is the primary model).
    if starts_with_ci(model, "claude-opus-4") {
        // Opus 4.5 / 4.6: $5.00 / $25.00 / $6.25 / $0.50 per million tokens → micro-USD
        // (Opus 4.1 and earlier had $15/$75 but those models are retired/deprecated.)
        Some(ModelPricing {
            input_micro_usd_per_million: 5_000_000,
            output_micro_usd_per_million: 25_000_000,
            cache_read_micro_usd_per_million: 500_000,
            cache_write_micro_usd_per_million: 6_250_000,
        })
    } else if starts_with_ci(model, "claude-sonnet-4") {
        // Sonnet 4.x: $3.00 / $15.00 / $3.75 / $0.30 per million tokens → micro-USD
        Some(ModelPricing {
            input_micro_usd_per_million: 3_000_000,
            output_micro_usd_per_million: 15_000_000,
            cache_read_micro_usd_per_million: 300_000,
            cache_write_micro_usd_per_million: 3_750_000,
        })
    } else if starts_with_ci(model, "claude-haiku-4") {
        // Haiku 4.5: $1.00 / $5.00 / $1.25 / $0.10 per million tokens → micro-USD
        Some(ModelPricing {
            input_micro_usd_per_million: 1_000_000,
            output_micro_usd_per_million: 5_000_000,
            cache_read_micro_usd_per_million: 100_000,
            cache_write_micro_usd_per_million: 1_250_000,
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cost_calculation_sonnet_4() {
        let usage = UsageInfo {
            prompt_tokens: 1000,
            completion_tokens: 500,
            cache_read_tokens: Some(200),
            cache_write_tokens: Some(100),
            context_tokens: None,
            cost: None,
        };
        let pricing = model_pricing("claude-sonnet-4-6").unwrap();
        let cost = pricing.cost_for(&usage);
        // Input: 1000/1M * $3.00 = $0.003 = 3000 micro-USD
        assert_eq!(cost.input_cost_micro_usd, 3000);
        assert!((cost.input_cost_usd() - 0.003).abs() < 1e-9);
        // Output: 500/1M * $15.00 = $0.0075 = 7500 micro-USD
        assert_eq!(cost.output_cost_micro_usd, 7500);
        assert!((cost.output_cost_usd() - 0.0075).abs() < 1e-9);
        // Cache read: 200/1M * $0.30 = $0.00006 = 60 micro-USD (integer: 200*300_000/1_000_000=60)
        assert_eq!(cost.cache_read_cost_micro_usd, 60);
        // Cache write: 100/1M * $3.75 = $0.000375 = 375 micro-USD (100*3_750_000/1_000_000=375)
        assert_eq!(cost.cache_write_cost_micro_usd, 375);
        // Total
        assert_eq!(cost.total_cost_micro_usd, 3000 + 7500 + 60 + 375);
    }

    #[test]
    fn test_cost_calculation_opus_4() {
        let usage = UsageInfo {
            prompt_tokens: 1_000_000,
            completion_tokens: 100_000,
            cache_read_tokens: None,
            cache_write_tokens: None,
            context_tokens: None,
            cost: None,
        };
        let pricing = model_pricing("claude-opus-4-6").unwrap();
        let cost = pricing.cost_for(&usage);
        // Opus 4.5/4.6: $5.00/MTok input (not $15 — that was Opus 4.1 and earlier)
        // Input: 1M/1M * $5.00 = $5.00 = 5_000_000 micro-USD
        assert_eq!(cost.input_cost_micro_usd, 5_000_000);
        assert!((cost.input_cost_usd() - 5.0).abs() < 1e-6);
        // Output: 100K/1M * $25.00 = $2.50 = 2_500_000 micro-USD
        assert_eq!(cost.output_cost_micro_usd, 2_500_000);
        assert!((cost.output_cost_usd() - 2.5).abs() < 1e-6);
    }

    #[test]
    fn test_model_pricing_unknown_returns_none() {
        assert!(model_pricing("gpt-4o").is_none());
        assert!(model_pricing("unknown-model").is_none());
        assert!(model_pricing("claude-3-5-sonnet-20241022").is_none());
        assert!(model_pricing("claude-3-5-haiku-20241022").is_none());
        assert!(model_pricing("claude-3-7-sonnet-20250219").is_none());
    }

    #[test]
    fn test_model_pricing_known_models() {
        assert!(model_pricing("claude-sonnet-4-6").is_some());
        assert!(model_pricing("claude-opus-4-6").is_some());
        assert!(model_pricing("claude-haiku-4-5").is_some());
        assert!(model_pricing("claude-haiku-4-5-20251001").is_some());
        // Prefix match covers dated variants of all three supported families
        assert!(model_pricing("claude-sonnet-4-6").is_some());
        assert!(model_pricing("claude-opus-4-20250514").is_some());
        assert!(model_pricing("claude-haiku-4-20250514").is_some());
        // Case-insensitive
        assert!(model_pricing("Claude-Sonnet-4-6").is_some());
        assert!(model_pricing("CLAUDE-OPUS-4-6").is_some());
        assert!(model_pricing("Claude-Haiku-4-5").is_some());
    }

    #[test]
    fn test_starts_with_ci() {
        assert!(starts_with_ci("Claude-Sonnet-4-6", "claude-sonnet-4"));
        assert!(starts_with_ci("CLAUDE-OPUS-4", "claude-opus-4"));
        assert!(!starts_with_ci("claude-3-5-sonnet", "claude-sonnet-4"));
        assert!(!starts_with_ci("short", "claude-sonnet-4"));
    }

    #[test]
    fn stop_reason_parse_all_variants() {
        assert_eq!(StopReason::parse("end_turn"), StopReason::EndTurn);
        assert_eq!(StopReason::parse("max_tokens"), StopReason::MaxTokens);
        assert_eq!(
            StopReason::parse("model_context_window_exceeded"),
            StopReason::MaxTokens
        );
        assert_eq!(StopReason::parse("tool_use"), StopReason::ToolUse);
        assert_eq!(StopReason::parse("refusal"), StopReason::Refusal);
        assert_eq!(StopReason::parse("pause_turn"), StopReason::EndTurn);
        assert_eq!(StopReason::parse("stop_sequence"), StopReason::EndTurn);
        assert_eq!(StopReason::parse("sensitive"), StopReason::Error);
        assert_eq!(StopReason::parse("error"), StopReason::Error);
        assert_eq!(StopReason::parse("aborted"), StopReason::Aborted);
        assert_eq!(
            StopReason::parse("custom_stop"),
            StopReason::Unknown("custom_stop".into())
        );
    }

    #[test]
    fn stop_reason_as_str_and_display_round_trip() {
        assert_eq!(StopReason::EndTurn.as_str(), "end_turn");
        assert_eq!(StopReason::MaxTokens.as_str(), "max_tokens");
        assert_eq!(StopReason::ToolUse.as_str(), "tool_use");
        assert_eq!(StopReason::Refusal.as_str(), "refusal");
        assert_eq!(StopReason::Error.as_str(), "error");
        assert_eq!(StopReason::Aborted.as_str(), "aborted");
        assert_eq!(StopReason::Unknown("weird".into()).as_str(), "weird");
        assert_eq!(StopReason::ToolUse.to_string(), "tool_use");
    }
}
