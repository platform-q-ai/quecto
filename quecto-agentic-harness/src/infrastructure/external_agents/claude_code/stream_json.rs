//! Decode one line of `claude -p --output-format stream-json` into the
//! domain's [`ExternalAgentEvent`]s (#2285).
//!
//! One line can carry several events (an assistant line's content blocks
//! and its `error`, a user line's tool results), so a line decodes to a
//! list. Every field another CLI version may omit is optional here. An
//! event type this codec does not know becomes
//! [`ExternalAgentEvent::Unknown`] and is logged; nothing panics. A line
//! that is not a JSON object, or a known event whose fields have the
//! wrong types, is a [`StreamJsonError`].

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

use crate::domain::external_agent::stream::{
    AssistantContent, BackgroundTask, ExternalAgentEvent, InitEvent, McpServerStatus, ModelUsage,
    PermissionDenial, RateLimitInfo, RateLimitStatus, RateLimitWindow, ResultEvent,
    TaskNotification, TaskStarted, TokenCounts, ToolResultEvent,
};

/// The `non_execution_kind` of a tool call a permission rule refused.
const PERMISSION_RULE: &str = "permission-rule";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum StreamJsonError {
    #[error("stream-json line is not JSON: {0}")]
    NotJson(String),
    #[error("stream-json line is not a JSON object")]
    NotAnObject,
    #[error("stream-json `{kind}` event is malformed: {reason}")]
    Malformed { kind: String, reason: String },
}

/// Decode one NDJSON line.
pub fn decode_line(line: &str) -> Result<Vec<ExternalAgentEvent>, StreamJsonError> {
    let value: Value =
        serde_json::from_str(line).map_err(|err| StreamJsonError::NotJson(err.to_string()))?;
    let Value::Object(object) = &value else {
        return Err(StreamJsonError::NotAnObject);
    };
    let field = |name| {
        object
            .get(name)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let (kind, subtype) = (field("type"), field("subtype"));
    let kind = kind.as_str();
    let events = match (kind, subtype.as_str()) {
        ("system", "init") => vec![ExternalAgentEvent::Init(init(parse(kind, value)?))],
        ("system", "thinking_tokens") => {
            let wire: WireThinkingTokens = parse(kind, value)?;
            vec![ExternalAgentEvent::ThinkingTokens {
                estimated_tokens: wire.estimated_tokens,
            }]
        }
        ("system", "task_started") => vec![task_started(parse(kind, value)?)],
        ("system", "task_notification") => vec![task_notification(parse(kind, value)?)],
        ("system", "background_tasks_changed") => {
            vec![background_tasks(parse(kind, value)?)]
        }
        ("assistant", _) => assistant(parse(kind, value)?),
        ("user", _) => user(parse(kind, value)?),
        ("rate_limit_event", _) => vec![rate_limit(parse(kind, value)?)],
        ("result", _) => vec![ExternalAgentEvent::Result(result(parse(kind, value)?))],
        _ => vec![unknown(kind, &subtype)],
    };
    Ok(events)
}

fn parse<T: for<'de> Deserialize<'de>>(kind: &str, value: Value) -> Result<T, StreamJsonError> {
    serde_json::from_value(value).map_err(|err| StreamJsonError::Malformed {
        kind: kind.to_string(),
        reason: err.to_string(),
    })
}

fn unknown(kind: &str, subtype: &str) -> ExternalAgentEvent {
    let kind = match subtype {
        "" => kind.to_string(),
        subtype => format!("{kind}/{subtype}"),
    };
    tracing::warn!(kind = %kind, "claude stream-json: unknown event type");
    ExternalAgentEvent::Unknown { kind }
}

#[derive(Deserialize)]
struct WireInit {
    session_id: Option<String>,
    model: Option<String>,
    #[serde(default)]
    tools: Vec<String>,
    #[serde(default)]
    mcp_servers: Vec<WireMcpServer>,
    #[serde(rename = "apiKeySource")]
    api_key_source: Option<String>,
    #[serde(rename = "permissionMode")]
    permission_mode: Option<String>,
}

#[derive(Deserialize)]
struct WireMcpServer {
    name: String,
    #[serde(default)]
    status: String,
}

fn init(wire: WireInit) -> InitEvent {
    InitEvent {
        session_id: wire.session_id,
        model: wire.model,
        tools: wire.tools,
        mcp_servers: wire
            .mcp_servers
            .into_iter()
            .map(|server| McpServerStatus {
                name: server.name,
                status: server.status,
            })
            .collect(),
        api_key_source: wire.api_key_source,
        permission_mode: wire.permission_mode,
    }
}

#[derive(Deserialize)]
struct WireThinkingTokens {
    estimated_tokens: Option<u64>,
}

#[derive(Deserialize)]
struct WireTaskStarted {
    task_id: String,
    tool_use_id: Option<String>,
    description: Option<String>,
    #[serde(default)]
    is_backgrounded: bool,
}

fn task_started(wire: WireTaskStarted) -> ExternalAgentEvent {
    ExternalAgentEvent::TaskStarted(TaskStarted {
        task_id: wire.task_id,
        tool_use_id: wire.tool_use_id,
        description: wire.description,
        is_backgrounded: wire.is_backgrounded,
    })
}

#[derive(Deserialize)]
struct WireTaskNotification {
    task_id: String,
    tool_use_id: Option<String>,
    status: Option<String>,
    summary: Option<String>,
}

fn task_notification(wire: WireTaskNotification) -> ExternalAgentEvent {
    ExternalAgentEvent::TaskNotification(TaskNotification {
        task_id: wire.task_id,
        tool_use_id: wire.tool_use_id,
        status: wire.status,
        summary: wire.summary,
    })
}

#[derive(Deserialize)]
struct WireBackgroundTasks {
    #[serde(default)]
    tasks: Vec<WireBackgroundTask>,
}

#[derive(Deserialize)]
struct WireBackgroundTask {
    task_id: String,
    description: Option<String>,
}

fn background_tasks(wire: WireBackgroundTasks) -> ExternalAgentEvent {
    ExternalAgentEvent::BackgroundTasksChanged {
        tasks: wire
            .tasks
            .into_iter()
            .map(|task| BackgroundTask {
                task_id: task.task_id,
                description: task.description,
            })
            .collect(),
    }
}

#[derive(Deserialize)]
struct WireAssistant {
    message: WireAssistantMessage,
    error: Option<String>,
}

#[derive(Deserialize)]
struct WireAssistantMessage {
    id: Option<String>,
    #[serde(default)]
    content: Vec<Value>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WireAssistantBlock {
    Text {
        #[serde(default)]
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        #[serde(default)]
        input: Value,
    },
    Thinking {
        #[serde(default)]
        thinking: String,
    },
}

fn assistant(wire: WireAssistant) -> Vec<ExternalAgentEvent> {
    let message_id = wire.message.id;
    let mut events: Vec<ExternalAgentEvent> = wire
        .message
        .content
        .into_iter()
        .map(|block| assistant_block(message_id.as_deref(), block))
        .collect();
    if let Some(kind) = wire.error {
        events.push(ExternalAgentEvent::AssistantError { message_id, kind });
    }
    events
}

fn assistant_block(message_id: Option<&str>, block: Value) -> ExternalAgentEvent {
    let block_kind = block
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    match (
        message_id,
        serde_json::from_value::<WireAssistantBlock>(block),
    ) {
        (Some(message_id), Ok(block)) => ExternalAgentEvent::AssistantBlock {
            message_id: message_id.to_string(),
            block: match block {
                WireAssistantBlock::Text { text } => AssistantContent::Text(text),
                WireAssistantBlock::ToolUse { id, name, input } => {
                    AssistantContent::ToolUse { id, name, input }
                }
                WireAssistantBlock::Thinking { thinking } => AssistantContent::Thinking {
                    text: Some(thinking).filter(|text| !text.trim().is_empty()),
                },
            },
        },
        _ => unknown("assistant", &block_kind),
    }
}

#[derive(Deserialize)]
struct WireUser {
    message: WireUserMessage,
    #[serde(default)]
    tool_result_meta: Vec<WireToolResultMeta>,
}

#[derive(Deserialize)]
struct WireUserMessage {
    #[serde(default)]
    content: Value,
}

#[derive(Deserialize)]
struct WireToolResultMeta {
    id: Option<String>,
    non_execution_kind: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WireUserBlock {
    ToolResult {
        tool_use_id: String,
        #[serde(default)]
        content: Value,
        #[serde(default)]
        is_error: bool,
    },
    Text {
        #[serde(default)]
        text: String,
    },
}

fn user(wire: WireUser) -> Vec<ExternalAgentEvent> {
    let denied: Vec<String> = wire
        .tool_result_meta
        .into_iter()
        .filter(|meta| meta.non_execution_kind.as_deref() == Some(PERMISSION_RULE))
        .filter_map(|meta| meta.id)
        .collect();
    match wire.message.content {
        Value::String(text) => vec![ExternalAgentEvent::UserText { text }],
        Value::Array(blocks) => blocks
            .into_iter()
            .map(|block| user_block(block, &denied))
            .collect(),
        _ => vec![unknown("user", "")],
    }
}

fn user_block(block: Value, denied: &[String]) -> ExternalAgentEvent {
    let block_kind = block
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    match serde_json::from_value::<WireUserBlock>(block) {
        Ok(WireUserBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        }) => ExternalAgentEvent::ToolResult(ToolResultEvent {
            permission_denied: denied.contains(&tool_use_id),
            tool_use_id,
            content,
            is_error,
        }),
        Ok(WireUserBlock::Text { text }) => ExternalAgentEvent::UserText { text },
        Err(_) => unknown("user", &block_kind),
    }
}

#[derive(Deserialize)]
struct WireRateLimitEvent {
    rate_limit_info: WireRateLimitInfo,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireRateLimitInfo {
    #[serde(default)]
    status: String,
    rate_limit_type: Option<String>,
    utilization: Option<f64>,
    resets_at: Option<i64>,
    is_using_overage: Option<bool>,
    #[serde(default)]
    unified_windows: BTreeMap<String, WireRateLimitWindow>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireRateLimitWindow {
    utilization: Option<f64>,
    resets_at: Option<i64>,
}

fn rate_limit(wire: WireRateLimitEvent) -> ExternalAgentEvent {
    let info = wire.rate_limit_info;
    ExternalAgentEvent::RateLimit(RateLimitInfo {
        status: RateLimitStatus::parse(&info.status),
        limit_type: info.rate_limit_type,
        utilization: info.utilization,
        resets_at: info.resets_at,
        windows: info
            .unified_windows
            .into_iter()
            .map(|(name, window)| RateLimitWindow {
                name,
                utilization: window.utilization,
                resets_at: window.resets_at,
            })
            .collect(),
        using_overage: info.is_using_overage,
    })
}

#[derive(Deserialize)]
struct WireResult {
    is_error: Option<bool>,
    terminal_reason: Option<String>,
    stop_reason: Option<String>,
    api_error_status: Option<Value>,
    result: Option<String>,
    #[serde(default)]
    usage: WireUsage,
    total_cost_usd: Option<f64>,
    #[serde(rename = "modelUsage", default)]
    model_usage: BTreeMap<String, WireModelUsage>,
    #[serde(default)]
    permission_denials: Vec<WirePermissionDenial>,
    num_turns: Option<u32>,
    duration_ms: Option<u64>,
}

#[derive(Deserialize, Default)]
struct WireUsage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireModelUsage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    #[serde(rename = "costUSD")]
    cost_usd: Option<f64>,
}

#[derive(Deserialize)]
struct WirePermissionDenial {
    tool_name: String,
    tool_use_id: String,
    #[serde(default)]
    tool_input: Value,
}

fn result(wire: WireResult) -> ResultEvent {
    ResultEvent {
        is_error: wire.is_error,
        terminal_reason: wire.terminal_reason,
        stop_reason: wire.stop_reason,
        api_error_status: wire.api_error_status.as_ref().and_then(http_status),
        result_text: wire.result,
        usage: tokens(
            wire.usage.input_tokens,
            wire.usage.output_tokens,
            wire.usage.cache_read_input_tokens,
            wire.usage.cache_creation_input_tokens,
        ),
        total_cost_usd: wire.total_cost_usd,
        model_usage: wire
            .model_usage
            .into_iter()
            .map(|(model, usage)| ModelUsage {
                model,
                tokens: tokens(
                    usage.input_tokens,
                    usage.output_tokens,
                    usage.cache_read_input_tokens,
                    usage.cache_creation_input_tokens,
                ),
                cost_usd: usage.cost_usd,
            })
            .collect(),
        permission_denials: wire
            .permission_denials
            .into_iter()
            .map(|denial| PermissionDenial {
                tool_name: denial.tool_name,
                tool_use_id: denial.tool_use_id,
                tool_input: denial.tool_input,
            })
            .collect(),
        num_turns: wire.num_turns,
        duration_ms: wire.duration_ms,
    }
}

/// Token counts from the wire; an absent or `null` count is zero.
fn tokens(
    input: Option<u64>,
    output: Option<u64>,
    cache_read: Option<u64>,
    cache_write: Option<u64>,
) -> TokenCounts {
    TokenCounts {
        input: input.unwrap_or(0),
        output: output.unwrap_or(0),
        cache_read: cache_read.unwrap_or(0),
        cache_write: cache_write.unwrap_or(0),
    }
}

/// An `api_error_status` as an HTTP status: a number, or a numeric string.
fn http_status(value: &Value) -> Option<u16> {
    match value {
        Value::Number(number) => number.as_u64().and_then(|n| u16::try_from(n).ok()),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
#[path = "stream_json_tests.rs"]
mod tests;
