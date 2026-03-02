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
}
