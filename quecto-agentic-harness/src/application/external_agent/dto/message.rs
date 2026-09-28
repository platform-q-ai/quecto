//! The member's conversation as the projection records it (#2285).

use serde_json::Value;

/// What the member is doing, as `get_state` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExecutionState {
    #[default]
    Idle,
    Thinking,
    Streaming,
    RunningTool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
    Tool,
}

/// One tool call of an assistant message.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectedToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// One message of the member's conversation.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectedMessage {
    pub ordinal: u64,
    pub role: MessageRole,
    pub content: String,
    pub tool_calls: Vec<ProjectedToolCall>,
    /// Thinking text; `None` while the CLI redacts it.
    pub thinking: Option<String>,
    /// The API message id an assistant message was grouped by.
    pub api_message_id: Option<String>,
    /// The tool call a tool message answers.
    pub tool_call_id: Option<String>,
    pub is_error: bool,
    /// A tool message whose call a permission rule (a hook) refused.
    pub permission_denied: bool,
}

impl ProjectedMessage {
    pub(crate) fn new(ordinal: u64, role: MessageRole) -> Self {
        Self {
            ordinal,
            role,
            content: String::new(),
            tool_calls: Vec::new(),
            thinking: None,
            api_message_id: None,
            tool_call_id: None,
            is_error: false,
            permission_denied: false,
        }
    }
}
