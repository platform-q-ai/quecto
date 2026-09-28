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

/// A tool result's content is stored up to this many bytes (cut on a
/// character boundary, then [`TRUNCATION_MARKER`]); the rest is dropped and
/// its full length kept in [`ProjectedMessage::truncated_from_bytes`].
pub const TOOL_RESULT_CONTENT_BYTES: usize = 64 * 1024;

/// Appended to a tool result's stored content when it was cut.
pub const TRUNCATION_MARKER: &str = "\n[… tool result truncated]";

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
    /// The original length of content cut to [`TOOL_RESULT_CONTENT_BYTES`].
    pub truncated_from_bytes: Option<usize>,
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
            truncated_from_bytes: None,
        }
    }
}
