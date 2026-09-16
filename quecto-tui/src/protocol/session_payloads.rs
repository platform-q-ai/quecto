//! Typed protocol values for TUI session-related wire payloads.
//!
//! The infrastructure client still receives raw JSON from the UDS protocol, but
//! presentation code should not hand-parse those protocol shapes in render/app
//! paths. These mappers keep that translation in the protocol layer.

use serde::Deserialize;
use serde_json::Value as JsonValue;

#[derive(Debug, Deserialize)]
struct ResumedMessagesWire {
    messages: Vec<ResumedMessageWire>,
}

#[derive(Debug, Deserialize)]
struct ResumedMessageWire {
    #[serde(default)]
    role: String,
    #[serde(default)]
    content: String,
    id: Option<String>,
    #[serde(default)]
    collapsed: bool,
    #[serde(rename = "contentLength")]
    content_len: Option<usize>,
    #[serde(default, alias = "tool_calls", rename = "toolCalls")]
    tool_calls: Vec<ToolCallWire>,
    #[serde(alias = "tool_call_id", rename = "toolCallId")]
    tool_call_id: Option<String>,
    #[serde(alias = "tool_name", rename = "toolName")]
    tool_name: Option<String>,
    #[serde(default, alias = "is_error", rename = "isError")]
    is_error: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct ToolCallWire {
    id: Option<String>,
    name: Option<String>,
    arguments: Option<JsonValue>,
    function: Option<ToolFunctionWire>,
}

#[derive(Debug, Clone, Deserialize)]
struct ToolFunctionWire {
    name: Option<String>,
    arguments: Option<JsonValue>,
}

/// Parsed session statistics used by chat status lines and footer indicators.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionStats {
    pub session_key: String,
    pub total_messages: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub total_tokens: u64,
    pub cost_micro_usd: u64,
    pub cost: f64,
    pub cache_hit_ratio: Option<f64>,
    pub context_usage: Option<(u64, usize)>,
}

/// A persisted session entry suitable for the resume selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeSessionSummary {
    pub key: String,
    pub title: String,
    pub message_count: u64,
    pub updated_unix_secs: Option<u64>,
    pub execution_location: Option<String>,
    pub repository_label: Option<String>,
    pub is_local: Option<bool>,
    pub legacy_unscoped: bool,
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
fn optional_usize_field(data: &JsonValue, key: &str) -> Option<usize> {
    data.get(key)
        .and_then(|v| v.as_u64())
        .and_then(|n| usize::try_from(n).ok())
}

fn token_field(data: &JsonValue, key: &str) -> u64 {
    data.get("tokens")
        .and_then(|t| t.get(key))
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
}

fn legacy_cost_to_micro_usd(dollars: f64) -> u64 {
    if dollars.is_finite() && dollars > 0.0 {
        (dollars * 1_000_000.0).round() as u64
    } else {
        0
    }
}

fn cost_micro_usd(data: &JsonValue) -> u64 {
    if let Some(value) = data.get("costMicroUsd") {
        value.as_u64().unwrap_or(0)
    } else {
        data.get("cost")
            .and_then(|v| v.as_f64())
            .map(legacy_cost_to_micro_usd)
            .unwrap_or(0)
    }
}

pub fn parse_session_stats(data: &JsonValue) -> SessionStats {
    let context_tokens = data.get("contextTokens").and_then(|v| v.as_u64());
    let max_context_tokens = optional_usize_field(data, "maxContextTokens");
    let cost_micro_usd = cost_micro_usd(data);

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
        input_tokens: token_field(data, "input"),
        output_tokens: token_field(data, "output"),
        cache_read_tokens: token_field(data, "cacheRead"),
        cache_write_tokens: token_field(data, "cacheWrite"),
        total_tokens: token_field(data, "total"),
        cost_micro_usd,
        cost: cost_micro_usd as f64 / 1_000_000.0,
        cache_hit_ratio: data.get("cacheHitRatio").and_then(|v| v.as_f64()),
        context_usage: context_tokens.zip(max_context_tokens),
    }
}

/// Parse a `list_sessions` response payload into selector summaries. Entries
/// without a human-readable title/name are skipped because they cannot be shown
/// or selected meaningfully.
pub fn parse_resume_sessions(data: &JsonValue) -> Vec<ResumeSessionSummary> {
    session_values(data)
        .iter()
        .filter_map(|session| {
            let title = session
                .get("title")
                .or_else(|| session.get("name"))
                .and_then(|v| v.as_str())?;
            let key = session.get("key").and_then(|v| v.as_str()).unwrap_or(title);
            let scope = session.get("scope");
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
                execution_location: scope
                    .and_then(|v| v.get("executionLocation"))
                    .and_then(|v| v.as_str())
                    .map(str::to_owned),
                repository_label: scope
                    .and_then(|v| v.get("repositoryLabel"))
                    .and_then(|v| v.as_str())
                    .map(str::to_owned),
                is_local: scope
                    .and_then(|v| v.get("isLocal"))
                    .and_then(|v| v.as_bool()),
                legacy_unscoped: scope.and_then(|v| v.get("kind")).and_then(|v| v.as_str())
                    == Some("legacy_unscoped"),
            })
        })
        .collect()
}

/// Parse a `get_messages` payload after session resume into displayable chat
/// messages. Unknown roles and empty assistant messages are intentionally
/// omitted to preserve previous TUI behavior.
pub fn parse_resumed_messages(
    data: &JsonValue,
) -> Result<Vec<ResumedChatMessage>, ResumeMessagesError> {
    if data.get("messages").is_none() {
        return Err(ResumeMessagesError::MissingMessages);
    }
    let wire: ResumedMessagesWire =
        serde_json::from_value(data.clone()).map_err(|_| ResumeMessagesError::MalformedMessages)?;
    Ok(wire
        .messages
        .into_iter()
        .flat_map(|message| {
            let content = message.content.clone();
            let id = message.id.clone();
            let stub = message.collapsed;
            let content_len = message.content_len;
            match message.role.as_str() {
                // Sub-agent notes are user-role turns on the wire but operator
                // status in the UI; not part of the resumed transcript (#1338).
                "user" if super::presentation_payloads::is_subagent_note(&content) => Vec::new(),
                "user" => vec![ResumedChatMessage::User {
                    text: content,
                    id,
                    stub,
                    content_len,
                }],
                "assistant" => parse_assistant_resume_messages(
                    message.tool_calls.clone(),
                    content,
                    id,
                    stub,
                    content_len,
                ),
                "tool" => parse_tool_result_resume_message(message, content)
                    .into_iter()
                    .collect(),
                _ => Vec::new(),
            }
        })
        .collect())
}

fn parse_assistant_resume_messages(
    tool_calls: Vec<ToolCallWire>,
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
        tool_calls
            .into_iter()
            .filter_map(parse_tool_call_resume_message),
    );
    resumed
}

fn parse_tool_call_resume_message(call: ToolCallWire) -> Option<ResumedChatMessage> {
    let tool_call_id = call.id.filter(|id| !id.is_empty())?;
    let tool_name = call
        .name
        .or_else(|| call.function.as_ref().and_then(|f| f.name.clone()))
        .unwrap_or_else(|| "tool".to_string());
    let args = call
        .arguments
        .or_else(|| call.function.and_then(|f| f.arguments))
        .as_ref()
        .map(json_string_or_raw)
        .unwrap_or_else(|| "{}".to_string());
    Some(ResumedChatMessage::ToolCall {
        tool_call_id: tool_call_id.to_string(),
        tool_name,
        args,
    })
}

fn parse_tool_result_resume_message(
    message: ResumedMessageWire,
    content: String,
) -> Option<ResumedChatMessage> {
    let tool_call_id = message.tool_call_id.filter(|id| !id.is_empty())?;
    let tool_name = message.tool_name;
    let is_error = message.is_error;
    Some(ResumedChatMessage::ToolResult {
        tool_call_id: tool_call_id.to_string(),
        tool_name,
        content,
        is_error,
    })
}

fn json_string_or_raw(value: &JsonValue) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

/// Whether the payload explicitly contained session entries, even if none are
/// resumable after parsing. Allows the presentation layer to keep its more specific
/// empty-vs-malformed user messages without parsing raw fields itself.
pub fn has_session_entries(data: &JsonValue) -> bool {
    !session_values(data).is_empty()
}

fn session_values(data: &JsonValue) -> &[JsonValue] {
    data.get("sessions")
        .and_then(|v| v.as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

#[cfg(test)]
#[path = "session_payloads_tests.rs"]
mod tests;
