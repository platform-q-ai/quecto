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
}

/// Token usage information from an LLM call.
#[derive(Debug, Clone)]
pub struct UsageInfo {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}
