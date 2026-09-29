//! Decode one line of `claude -p --output-format stream-json` into the
//! domain's [`ExternalAgentEvent`]s (#2285).
//!
//! One line can carry several events (an assistant line's content blocks
//! and its `error`, a user line's tool results), so a line decodes to a
//! list. Every field is read on its own: one of the wrong shape is absent,
//! never the loss of the event. A `result` is always a turn end; one whose
//! essentials are malformed is a failed one. An event type this codec does
//! not know, or a task event without its task id, becomes
//! [`ExternalAgentEvent::Unknown`], logged once per kind; nothing panics.
//! Only a line that is not a JSON object is a [`StreamJsonError`], and the
//! reader (S2) skips and logs it: one bad line never ends the stream.

use std::collections::BTreeSet;

use serde_json::Value;

use super::json_fields::{Object, count, flag, list, number, object, seconds, text, texts};
use super::result_json::result;
use crate::domain::external_agent::stream::{
    AssistantContent, BackgroundTask, ExternalAgentEvent, InitEvent, InterruptReceipt,
    McpServerStatus, RateLimitInfo, RateLimitStatus, RateLimitWindow, TaskNotification,
    TaskStarted, ToolResultEvent,
};

/// The `non_execution_kind` of a tool call a permission rule refused.
const PERMISSION_RULE: &str = "permission-rule";

/// Unknown kinds logged per decoder; past this, new kinds go unlogged.
const UNKNOWN_KINDS_LOGGED_CAPACITY: usize = 64;

/// A line that is not a JSON object. The reader skips and logs it; it is
/// never the end of the stream.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum StreamJsonError {
    #[error("stream-json line is not JSON: {0}")]
    NotJson(String),
    #[error("stream-json line is not a JSON object")]
    NotAnObject,
}

/// Decodes one process's stream, line by line.
#[derive(Debug, Default)]
pub struct StreamJsonDecoder {
    /// Assistant lines without a `message.id` so far.
    synthetic_ids: u64,
    unknown_kinds_logged: BTreeSet<String>,
}

impl StreamJsonDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decode one NDJSON line.
    pub fn decode_line(&mut self, line: &str) -> Result<Vec<ExternalAgentEvent>, StreamJsonError> {
        let value: Value =
            serde_json::from_str(line).map_err(|err| StreamJsonError::NotJson(err.to_string()))?;
        let Value::Object(line) = value else {
            return Err(StreamJsonError::NotAnObject);
        };
        let kind = text(&line, "type").unwrap_or_default();
        let subtype = text(&line, "subtype").unwrap_or_default();
        let events = match (kind.as_str(), subtype.as_str()) {
            ("system", "init") => vec![ExternalAgentEvent::Init(init(&line))],
            ("system", "thinking_tokens") => vec![ExternalAgentEvent::ThinkingTokens {
                estimated_tokens: count(&line, "estimated_tokens"),
            }],
            ("system", "task_started") => {
                vec![task_started(&line).unwrap_or_else(|| self.unknown(&kind, &subtype))]
            }
            ("system", "task_notification") => {
                vec![task_notification(&line).unwrap_or_else(|| self.unknown(&kind, &subtype))]
            }
            ("system", "background_tasks_changed") => vec![background_tasks(&line)],
            ("assistant", _) => self.assistant(&line),
            ("user", _) => self.user(&line),
            ("rate_limit_event", _) => vec![rate_limit(&line)],
            ("result", _) => vec![ExternalAgentEvent::Result(result(&line))],
            ("control_response", _) => vec![ExternalAgentEvent::InterruptAnswered(
                interrupt_receipt(&line),
            )],
            _ => vec![self.unknown(&kind, &subtype)],
        };
        Ok(events)
    }

    /// How many distinct unknown event kinds were logged.
    pub fn unknown_kinds_logged(&self) -> usize {
        self.unknown_kinds_logged.len()
    }

    fn unknown(&mut self, kind: &str, subtype: &str) -> ExternalAgentEvent {
        let kind = match subtype {
            "" => kind.to_string(),
            subtype => format!("{kind}/{subtype}"),
        };
        let room = self.unknown_kinds_logged.len() < UNKNOWN_KINDS_LOGGED_CAPACITY;
        if room && self.unknown_kinds_logged.insert(kind.clone()) {
            tracing::warn!(kind = %kind, "claude stream-json: unknown event type");
        }
        ExternalAgentEvent::Unknown { kind }
    }

    fn assistant(&mut self, line: &Object) -> Vec<ExternalAgentEvent> {
        let message = object(line, "message");
        let message_id = message.and_then(|m| text(m, "id"));
        // Blocks without an id still form one message per line.
        let group = match &message_id {
            Some(id) => id.clone(),
            None => {
                self.synthetic_ids += 1;
                format!("synthetic-{}", self.synthetic_ids)
            }
        };
        let blocks = message.map_or(&[][..], |m| list(m, "content"));
        let mut events: Vec<ExternalAgentEvent> = blocks
            .iter()
            .map(|block| self.assistant_block(&group, block))
            .collect();
        if let Some(kind) = text(line, "error") {
            events.push(ExternalAgentEvent::AssistantError { message_id, kind });
        }
        events
    }

    fn assistant_block(&mut self, group: &str, block: &Value) -> ExternalAgentEvent {
        let empty = Object::new();
        let block = block.as_object().unwrap_or(&empty);
        let kind = text(block, "type").unwrap_or_default();
        let content = match kind.as_str() {
            "text" => AssistantContent::Text(text(block, "text").unwrap_or_default()),
            "tool_use" => AssistantContent::ToolUse {
                id: text(block, "id").unwrap_or_default(),
                name: text(block, "name").unwrap_or_default(),
                input: block.get("input").cloned().unwrap_or(Value::Null),
            },
            "thinking" => AssistantContent::Thinking {
                text: text(block, "thinking").filter(|text| !text.trim().is_empty()),
            },
            "redacted_thinking" => AssistantContent::Thinking { text: None },
            other => return self.unknown("assistant", other),
        };
        ExternalAgentEvent::AssistantBlock {
            message_id: group.to_string(),
            block: content,
        }
    }

    fn user(&mut self, line: &Object) -> Vec<ExternalAgentEvent> {
        let denied: Vec<String> = list(line, "tool_result_meta")
            .iter()
            .filter_map(Value::as_object)
            .filter(|meta| text(meta, "non_execution_kind").as_deref() == Some(PERMISSION_RULE))
            .filter_map(|meta| text(meta, "id"))
            .collect();
        match object(line, "message").and_then(|m| m.get("content")) {
            Some(Value::String(text)) => vec![ExternalAgentEvent::UserText { text: text.clone() }],
            Some(Value::Array(blocks)) => blocks
                .iter()
                .map(|block| self.user_block(block, &denied))
                .collect(),
            _ => vec![self.unknown("user", "")],
        }
    }

    fn user_block(&mut self, block: &Value, denied: &[String]) -> ExternalAgentEvent {
        let empty = Object::new();
        let block = block.as_object().unwrap_or(&empty);
        match text(block, "type").unwrap_or_default().as_str() {
            "tool_result" => {
                let tool_use_id = text(block, "tool_use_id");
                ExternalAgentEvent::ToolResult(ToolResultEvent {
                    permission_denied: tool_use_id.as_ref().is_some_and(|id| denied.contains(id)),
                    tool_use_id,
                    content: block.get("content").cloned().unwrap_or(Value::Null),
                    is_error: tool_error(block.get("is_error")),
                })
            }
            "text" => ExternalAgentEvent::UserText {
                text: text(block, "text").unwrap_or_default(),
            },
            other => self.unknown("user", other),
        }
    }
}

/// A tool result's `is_error`: absent or `null` is no error, a boolean is
/// itself, and anything else is taken as an error.
fn tool_error(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(is_error)) => *is_error,
        Some(_) => true,
    }
}

/// A `control_response`: the answer to the interrupt, the only control
/// request a member sends (#2287). Accepted only as `subtype: success`;
/// what it withdrew is `response.cancelled` (absent from an older CLI's
/// empty answer).
fn interrupt_receipt(line: &Object) -> InterruptReceipt {
    let empty = Object::new();
    let response = object(line, "response").unwrap_or(&empty);
    match text(response, "subtype").as_deref() {
        Some("success") => InterruptReceipt {
            accepted: true,
            cancelled: object(response, "response")
                .map(|answer| {
                    list(answer, "cancelled")
                        .iter()
                        .filter_map(Value::as_str)
                        .filter(|id| !id.is_empty())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
        },
        _ => InterruptReceipt::default(),
    }
}

fn init(line: &Object) -> InitEvent {
    InitEvent {
        session_id: text(line, "session_id"),
        model: text(line, "model"),
        tools: texts(line, "tools"),
        mcp_servers: list(line, "mcp_servers")
            .iter()
            .filter_map(Value::as_object)
            .map(|server| McpServerStatus {
                name: text(server, "name").unwrap_or_default(),
                status: text(server, "status").unwrap_or_default(),
            })
            .collect(),
        api_key_source: text(line, "apiKeySource"),
        permission_mode: text(line, "permissionMode"),
        cli_version: None,
        capabilities: Vec::new(),
    }
}

/// A task event names its task; one that does not is `None`.
fn task_started(line: &Object) -> Option<ExternalAgentEvent> {
    Some(ExternalAgentEvent::TaskStarted(TaskStarted {
        task_id: text(line, "task_id")?,
        tool_use_id: text(line, "tool_use_id"),
        description: text(line, "description"),
        is_backgrounded: flag(line, "is_backgrounded").unwrap_or(false),
    }))
}

fn task_notification(line: &Object) -> Option<ExternalAgentEvent> {
    Some(ExternalAgentEvent::TaskNotification(TaskNotification {
        task_id: text(line, "task_id")?,
        tool_use_id: text(line, "tool_use_id"),
        status: text(line, "status"),
        summary: text(line, "summary"),
    }))
}

fn background_tasks(line: &Object) -> ExternalAgentEvent {
    ExternalAgentEvent::BackgroundTasksChanged {
        tasks: list(line, "tasks")
            .iter()
            .filter_map(Value::as_object)
            .filter_map(|task| {
                Some(BackgroundTask {
                    task_id: text(task, "task_id")?,
                    description: text(task, "description"),
                })
            })
            .collect(),
    }
}

fn rate_limit(line: &Object) -> ExternalAgentEvent {
    let empty = Object::new();
    let info = object(line, "rate_limit_info").unwrap_or(&empty);
    ExternalAgentEvent::RateLimit(RateLimitInfo {
        status: RateLimitStatus::parse(&text(info, "status").unwrap_or_default()),
        limit_type: text(info, "rateLimitType"),
        utilization: number(info, "utilization"),
        resets_at: seconds(info, "resetsAt"),
        windows: object(info, "unifiedWindows")
            .map(|windows| {
                windows
                    .iter()
                    .map(|(name, window)| {
                        let window = window.as_object().unwrap_or(&empty);
                        RateLimitWindow {
                            name: name.clone(),
                            utilization: number(window, "utilization"),
                            resets_at: seconds(window, "resetsAt"),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default(),
        using_overage: flag(info, "isUsingOverage"),
    })
}

#[cfg(test)]
#[path = "stream_json_tests.rs"]
mod tests;
