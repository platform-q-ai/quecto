//! Typed protocol values for TUI session-related wire payloads.
//!
//! The infrastructure client still receives raw JSON from the UDS protocol, but
//! presentation code should not hand-parse those protocol shapes in render/app
//! paths. These mappers keep that translation in the protocol layer.

/// Parsed session statistics used by chat status lines and footer indicators.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionStats {
    pub session_key: String,
    pub total_messages: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost: f64,
    pub context_usage: Option<(u64, usize)>,
}

/// A persisted session entry suitable for the resume selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeSessionSummary {
    pub key: String,
    pub title: String,
    pub message_count: u64,
    pub updated_unix_secs: Option<u64>,
}

/// Displayable chat messages from a resumed/backfilled session.
///
/// Carries the stable server message id and whether the body is a ladder-demoted
/// stub (#1061), so the TUI can render a stub in place and recall its full body
/// on demand via `get_message`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumedChatMessage {
    User {
        text: String,
        id: Option<String>,
        stub: bool,
        content_len: Option<usize>,
    },
    Assistant {
        text: String,
        id: Option<String>,
        stub: bool,
        content_len: Option<usize>,
    },
    ToolCall {
        tool_call_id: String,
        tool_name: String,
        args: String,
    },
    ToolResult {
        tool_call_id: String,
        tool_name: Option<String>,
        content: String,
        is_error: bool,
    },
}

/// Why a resumed-session messages payload could not be used safely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeMessagesError {
    MissingMessages,
    MalformedMessages,
}

impl ResumeMessagesError {
    pub fn description(self) -> &'static str {
        match self {
            Self::MissingMessages => "missing messages array",
            Self::MalformedMessages => "messages field is not an array",
        }
    }
}

/// Parse a `get_session_stats` response payload into a typed value with the
/// same forgiving defaults the TUI historically used.
fn optional_usize_field(data: &serde_json::Value, key: &str) -> Option<usize> {
    data.get(key)
        .and_then(|v| v.as_u64())
        .and_then(|n| usize::try_from(n).ok())
}

pub fn parse_session_stats(data: &serde_json::Value) -> SessionStats {
    let context_tokens = data.get("contextTokens").and_then(|v| v.as_u64());
    let max_context_tokens = optional_usize_field(data, "maxContextTokens");

    SessionStats {
        session_key: data
            .get("sessionKey")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string(),
        total_messages: data
            .get("totalMessages")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        input_tokens: data
            .get("tokens")
            .and_then(|t| t.get("input"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        output_tokens: data
            .get("tokens")
            .and_then(|t| t.get("output"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        cost: data.get("cost").and_then(|v| v.as_f64()).unwrap_or(0.0),
        context_usage: context_tokens.zip(max_context_tokens),
    }
}

/// Parse a `list_sessions` response payload into selector summaries. Entries
/// without a human-readable title/name are skipped because they cannot be shown
/// or selected meaningfully.
pub fn parse_resume_sessions(data: &serde_json::Value) -> Vec<ResumeSessionSummary> {
    session_values(data)
        .iter()
        .filter_map(|session| {
            let title = session
                .get("title")
                .or_else(|| session.get("name"))
                .and_then(|v| v.as_str())?;
            let key = session.get("key").and_then(|v| v.as_str()).unwrap_or(title);
            Some(ResumeSessionSummary {
                key: key.to_string(),
                title: title.to_string(),
                message_count: session
                    .get("messageCount")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                updated_unix_secs: session
                    .get("updatedUnixSecs")
                    .or_else(|| session.get("updatedAt"))
                    .and_then(|v| v.as_u64()),
            })
        })
        .collect()
}

/// Parse a `get_messages` payload after session resume into displayable chat
/// messages. Unknown roles and empty assistant messages are intentionally
/// omitted to preserve previous TUI behavior.
pub fn parse_resumed_messages(
    data: &serde_json::Value,
) -> Result<Vec<ResumedChatMessage>, ResumeMessagesError> {
    let messages = message_values(data)?;
    Ok(messages
        .iter()
        .flat_map(|message| {
            let role = message.get("role").and_then(|v| v.as_str()).unwrap_or("");
            let content = message
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let id = message
                .get("id")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            // `collapsed` marks a ladder-demoted stub whose full body is recallable
            // by id (#1061). Absent on older payloads → treated as a full message.
            let stub = message
                .get("collapsed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let content_len = optional_usize_field(message, "contentLength");
            match role {
                "user" => vec![ResumedChatMessage::User {
                    text: content,
                    id,
                    stub,
                    content_len,
                }],
                "assistant" => {
                    parse_assistant_resume_messages(message, content, id, stub, content_len)
                }
                "tool" => parse_tool_result_resume_message(message, content)
                    .into_iter()
                    .collect(),
                _ => Vec::new(),
            }
        })
        .collect())
}

fn parse_assistant_resume_messages(
    message: &serde_json::Value,
    content: String,
    id: Option<String>,
    stub: bool,
    content_len: Option<usize>,
) -> Vec<ResumedChatMessage> {
    let mut resumed = Vec::new();
    if !content.is_empty() {
        resumed.push(ResumedChatMessage::Assistant {
            text: content,
            id,
            stub,
            content_len,
        });
    }
    resumed.extend(
        message
            .get("toolCalls")
            .or_else(|| message.get("tool_calls"))
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .filter_map(parse_tool_call_resume_message),
    );
    resumed
}

fn parse_tool_call_resume_message(call: &serde_json::Value) -> Option<ResumedChatMessage> {
    let tool_call_id = call.get("id").and_then(|v| v.as_str()).unwrap_or("");
    if tool_call_id.is_empty() {
        return None;
    }
    let tool_name = call
        .get("name")
        .or_else(|| call.pointer("/function/name"))
        .and_then(|v| v.as_str())
        .unwrap_or("tool")
        .to_string();
    let args = call
        .get("arguments")
        .or_else(|| call.pointer("/function/arguments"))
        .map(json_string_or_raw)
        .unwrap_or_else(|| "{}".to_string());
    Some(ResumedChatMessage::ToolCall {
        tool_call_id: tool_call_id.to_string(),
        tool_name,
        args,
    })
}

fn parse_tool_result_resume_message(
    message: &serde_json::Value,
    content: String,
) -> Option<ResumedChatMessage> {
    let tool_call_id = message
        .get("toolCallId")
        .or_else(|| message.get("tool_call_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if tool_call_id.is_empty() {
        return None;
    }
    let tool_name = message
        .get("toolName")
        .or_else(|| message.get("tool_name"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let is_error = message
        .get("isError")
        .or_else(|| message.get("is_error"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    Some(ResumedChatMessage::ToolResult {
        tool_call_id: tool_call_id.to_string(),
        tool_name,
        content,
        is_error,
    })
}

fn json_string_or_raw(value: &serde_json::Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

/// Whether the payload explicitly contained session entries, even if none are
/// resumable after parsing. Allows the presentation layer to keep its more specific
/// empty-vs-malformed user messages without parsing raw fields itself.
pub fn has_session_entries(data: &serde_json::Value) -> bool {
    !session_values(data).is_empty()
}

fn session_values(data: &serde_json::Value) -> &[serde_json::Value] {
    data.get("sessions")
        .and_then(|v| v.as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn message_values(data: &serde_json::Value) -> Result<&[serde_json::Value], ResumeMessagesError> {
    let Some(messages) = data.get("messages") else {
        return Err(ResumeMessagesError::MissingMessages);
    };
    messages
        .as_array()
        .map(Vec::as_slice)
        .ok_or(ResumeMessagesError::MalformedMessages)
}

#[cfg(test)]
#[path = "session_payloads_tests.rs"]
mod tests;
