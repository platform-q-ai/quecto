/// A single message in a conversation.
#[derive(Debug, Clone)]
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
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            tool_calls: vec![],
            tool_call_id: None,
            turn: None,
            is_pinned: true,
            is_manifest: false,
            is_collapsed: false,
            tool_name: None,
            input_preview: None,
            spill_id: None,
            image_blocks: vec![],
            is_error: false,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            tool_calls: vec![],
            tool_call_id: None,
            turn: None,
            is_pinned: false,
            is_manifest: false,
            is_collapsed: false,
            tool_name: None,
            input_preview: None,
            spill_id: None,
            image_blocks: vec![],
            is_error: false,
        }
    }

    pub fn assistant(content: impl Into<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_calls,
            tool_call_id: None,
            turn: None,
            is_pinned: false,
            is_manifest: false,
            is_collapsed: false,
            tool_name: None,
            input_preview: None,
            spill_id: None,
            image_blocks: vec![],
            is_error: false,
        }
    }

    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_calls: vec![],
            tool_call_id: Some(tool_call_id.into()),
            turn: None,
            is_pinned: false,
            is_manifest: false,
            is_collapsed: false,
            tool_name: None,
            input_preview: None,
            spill_id: None,
            image_blocks: vec![],
            is_error: false,
        }
    }
}

/// The role of a message sender.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    System,
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
    /// Unknown stop reason (future-proofing).
    Unknown(String),
}

impl StopReason {
    /// Parse an Anthropic stop_reason string.
    pub fn from_anthropic(reason: &str) -> Self {
        match reason {
            "end_turn" => Self::EndTurn,
            "max_tokens" => Self::MaxTokens,
            "tool_use" => Self::ToolUse,
            "refusal" => Self::Refusal,
            "pause_turn" | "stop_sequence" => Self::EndTurn,
            "sensitive" => Self::Error,
            other => Self::Unknown(other.to_string()),
        }
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
    /// Per-call cost breakdown, if model pricing is available.
    pub cost: Option<CostInfo>,
}

/// Per-call cost breakdown calculated from token usage and model pricing.
#[derive(Debug, Clone, PartialEq)]
pub struct CostInfo {
    /// Cost of input tokens in USD.
    pub input_cost: f64,
    /// Cost of output tokens in USD.
    pub output_cost: f64,
    /// Cost of cache-read input tokens in USD.
    pub cache_read_cost: f64,
    /// Cost of cache-write input tokens in USD.
    pub cache_write_cost: f64,
    /// Total cost in USD (sum of all components).
    pub total_cost: f64,
}

/// Per-million-token pricing for a model.
#[derive(Debug, Clone, Copy)]
pub struct ModelPricing {
    /// Input token cost per million tokens.
    pub input_per_million: f64,
    /// Output token cost per million tokens.
    pub output_per_million: f64,
    /// Cache-read input token cost per million tokens.
    pub cache_read_per_million: f64,
    /// Cache-write input token cost per million tokens.
    pub cache_write_per_million: f64,
}

impl CostInfo {
    /// Calculate cost from usage data and model pricing.
    pub fn from_usage(usage: &UsageInfo, pricing: &ModelPricing) -> Self {
        let per_m = |tokens: u32, rate: f64| (tokens as f64 / 1_000_000.0) * rate;
        let input_cost = per_m(usage.prompt_tokens, pricing.input_per_million);
        let output_cost = per_m(usage.completion_tokens, pricing.output_per_million);
        let cache_read_cost = per_m(
            usage.cache_read_tokens.unwrap_or(0),
            pricing.cache_read_per_million,
        );
        let cache_write_cost = per_m(
            usage.cache_write_tokens.unwrap_or(0),
            pricing.cache_write_per_million,
        );
        Self {
            input_cost,
            output_cost,
            cache_read_cost,
            cache_write_cost,
            total_cost: input_cost + output_cost + cache_read_cost + cache_write_cost,
        }
    }
}

/// Look up pricing for a known model. Returns `None` for unknown models.
///
/// Only `claude-sonnet-4` and `claude-opus-4` families are tracked.
pub fn model_pricing(model: &str) -> Option<ModelPricing> {
    // Normalise to lowercase for matching.
    let m = model.to_ascii_lowercase();
    // Match on model family prefix (covers dated variants like claude-sonnet-4-5, claude-sonnet-4-6).
    if m.starts_with("claude-sonnet-4") {
        Some(ModelPricing {
            input_per_million: 3.0,
            output_per_million: 15.0,
            cache_read_per_million: 0.30,
            cache_write_per_million: 3.75,
        })
    } else if m.starts_with("claude-opus-4") {
        Some(ModelPricing {
            input_per_million: 15.0,
            output_per_million: 75.0,
            cache_read_per_million: 1.50,
            cache_write_per_million: 18.75,
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
            cost: None,
        };
        let pricing = model_pricing("claude-sonnet-4-6").unwrap();
        let cost = CostInfo::from_usage(&usage, &pricing);
        // Input: 1000/1M * $3.00 = $0.003
        assert!((cost.input_cost - 0.003).abs() < 1e-9);
        // Output: 500/1M * $15.00 = $0.0075
        assert!((cost.output_cost - 0.0075).abs() < 1e-9);
        // Cache read: 200/1M * $0.30 = $0.00006
        assert!((cost.cache_read_cost - 0.00006).abs() < 1e-9);
        // Cache write: 100/1M * $3.75 = $0.000375
        assert!((cost.cache_write_cost - 0.000375).abs() < 1e-9);
        // Total
        let expected_total = 0.003 + 0.0075 + 0.00006 + 0.000375;
        assert!((cost.total_cost - expected_total).abs() < 1e-9);
    }

    #[test]
    fn test_cost_calculation_opus_4() {
        let usage = UsageInfo {
            prompt_tokens: 1_000_000,
            completion_tokens: 100_000,
            cache_read_tokens: None,
            cache_write_tokens: None,
            cost: None,
        };
        let pricing = model_pricing("claude-opus-4-6").unwrap();
        let cost = CostInfo::from_usage(&usage, &pricing);
        // Input: 1M/1M * $15.00 = $15.00
        assert!((cost.input_cost - 15.0).abs() < 1e-6);
        // Output: 100K/1M * $75.00 = $7.50
        assert!((cost.output_cost - 7.5).abs() < 1e-6);
    }

    #[test]
    fn test_model_pricing_unknown_returns_none() {
        assert!(model_pricing("gpt-4o").is_none());
        assert!(model_pricing("unknown-model").is_none());
        assert!(model_pricing("claude-3-5-sonnet-20241022").is_none());
        assert!(model_pricing("claude-3-5-haiku-20241022").is_none());
        assert!(model_pricing("claude-haiku-4-20250514").is_none());
        assert!(model_pricing("claude-3-7-sonnet-20250219").is_none());
    }

    #[test]
    fn test_model_pricing_known_models() {
        assert!(model_pricing("claude-sonnet-4-6").is_some());
        assert!(model_pricing("claude-opus-4-6").is_some());
        // Prefix match covers all dated variants of the two supported families
        assert!(model_pricing("claude-sonnet-4-20250514").is_some());
        assert!(model_pricing("claude-opus-4-20250514").is_some());
    }
}
