//! UDS client — connects to a quecto agent over a Unix domain socket.
//!
//! Sends JSON-lines commands and receives JSON-lines events. The client
//! is async (tokio) and designed to run in a background task, feeding
//! events to the TUI's main render loop.

use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

/// Maximum line size from the agent (1 MiB, matching quecto's protocol limit).
const MAX_LINE_BYTES: usize = 1_048_576;

// ─── Protocol types (subset matching quecto's wire format) ────────────────────

/// A command sent from the TUI to the agent.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    Prompt {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        message: String,
        /// How to handle this prompt if the agent is already running.
        #[serde(rename = "streamingBehavior", skip_serializing_if = "Option::is_none")]
        streaming_behavior: Option<String>,
    },
    Steer {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        message: String,
    },
    FollowUp {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        message: String,
    },
    Abort {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    GetState {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    GetMessages {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    GetMessagesTail {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        count: usize,
    },
    GetSessionStats {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    SetModel {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        provider: Option<String>,
        #[serde(rename = "modelId", skip_serializing_if = "Option::is_none")]
        model_id: Option<String>,
    },
    ClearHistory {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    GetSubagents {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
}

/// An event received from the agent.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    AgentStart,
    AgentEnd {
        messages: Vec<serde_json::Value>,
    },
    Token {
        token: String,
    },
    TurnStart,
    TurnEnd {
        message: serde_json::Value,
        #[serde(rename = "toolResults", default)]
        tool_results: Vec<serde_json::Value>,
    },
    ToolExecutionStart {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        args: serde_json::Value,
    },
    ToolExecutionEnd {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        result: serde_json::Value,
        #[serde(rename = "isError")]
        is_error: bool,
    },
    Response {
        #[serde(default)]
        id: Option<String>,
        command: String,
        success: bool,
        #[serde(default)]
        data: Option<serde_json::Value>,
        #[serde(default)]
        error: Option<String>,
    },
    /// Request to execute a tool (routed to extension clients, not broadcast).
    ExecuteTool {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        arguments: String,
    },
    ExtensionsChanged {
        extensions: Vec<serde_json::Value>,
    },
    /// Subagent state changed — full list replacement (#525).
    SubagentStateChanged {
        subagents: Vec<SubagentInfoEvent>,
    },
    /// Workflow state changed — step checked/unchecked/reset (#563).
    WorkflowState {
        steps: Vec<serde_json::Value>,
        progress: serde_json::Value,
        #[serde(rename = "activeIssue", default)]
        active_issue: Option<serde_json::Value>,
    },
    /// Catch-all for unknown/future event types (forward-compatible).
    #[serde(other)]
    Unknown,
}

/// Wire-format subagent info from `subagent_state_changed` event (#524/#525).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentInfoEvent {
    pub agent_id: String,
    pub status: String,
    pub last_tool: Option<String>,
    pub last_error: Option<String>,
    pub pid: u32,
}

// ─── Result text extraction ───────────────────────────────────────────────────

/// Extract the first text content from a tool result JSON value.
///
/// The server sends tool results as:
/// ```json
/// {"content": [{"type": "text", "text": "..."}]}
/// ```
/// This function extracts the `text` field from the first text block.
/// Used by `app.rs` when handling `ToolExecutionEnd` events.
pub fn extract_result_text(result: &serde_json::Value) -> String {
    result
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|arr| {
            arr.iter()
                .filter_map(|v| v.get("text").and_then(|t| t.as_str()))
                .next()
        })
        .unwrap_or("")
        .to_string()
}

// ─── Client ───────────────────────────────────────────────────────────────────

/// Error type for client operations.
#[derive(Debug)]
pub enum ClientError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Disconnected,
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Json(e) => write!(f, "JSON error: {e}"),
            Self::Disconnected => write!(f, "disconnected from agent"),
        }
    }
}

impl From<std::io::Error> for ClientError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for ClientError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

/// A cloneable sender for commands to the agent.
///
/// Multiple tasks can hold a `CommandSender` to send commands concurrently.
#[derive(Clone)]
pub struct CommandSender {
    tx: mpsc::Sender<String>,
}

impl CommandSender {
    /// Send a command to the agent.
    pub async fn send(&mut self, cmd: &Command) -> Result<(), ClientError> {
        let mut json = serde_json::to_string(cmd)?;
        json.push('\n');
        self.tx
            .send(json)
            .await
            .map_err(|_| ClientError::Disconnected)
    }
}

/// A UDS client connection to a quecto agent.
///
/// The client provides:
/// - `send()` to send commands to the agent
/// - `recv()` to receive events via an mpsc channel
/// - `clone_sender()` to get a cloneable command sender for use in spawned tasks
///
/// The event reader and command writer run in background tokio tasks.
pub struct Client {
    /// Channel to send serialized command lines to the writer task.
    cmd_tx: mpsc::Sender<String>,
    /// Channel for receiving events from the background reader.
    event_rx: mpsc::Receiver<Event>,
}

impl Client {
    /// Connect to a quecto agent at the given socket path.
    pub async fn connect(socket_path: &Path) -> Result<Self, ClientError> {
        let stream = UnixStream::connect(socket_path).await?;
        let (read_half, mut write_half) = tokio::io::split(stream);

        // Command writer task: receives serialized JSON lines and writes them to the socket.
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<String>(64);
        tokio::spawn(async move {
            while let Some(line) = cmd_rx.recv().await {
                if write_half.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
                if write_half.flush().await.is_err() {
                    break;
                }
            }
        });

        let (tx, rx) = mpsc::channel(256);

        // Spawn background event reader
        tokio::spawn(async move {
            let mut reader = BufReader::new(read_half);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => break, // EOF — agent closed the connection
                    Ok(_) => {
                        // Enforce max line size to prevent OOM from malicious/buggy agents.
                        if line.len() > MAX_LINE_BYTES {
                            eprintln!(
                                "quecto-tui: dropping oversized line ({} bytes, max {})",
                                line.len(),
                                MAX_LINE_BYTES
                            );
                            continue;
                        }
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        match serde_json::from_str::<Event>(trimmed) {
                            Ok(event) => {
                                if tx.send(event).await.is_err() {
                                    break; // Receiver dropped
                                }
                            }
                            Err(e) => {
                                // Truncate logged content to avoid leaking sensitive data.
                                let preview_len = trimmed.len().min(200);
                                eprintln!(
                                    "quecto-tui: failed to parse agent event: {e} (line len: {}, preview: {}...)",
                                    trimmed.len(),
                                    &trimmed[..preview_len]
                                );
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("quecto-tui: error reading from agent socket: {e}");
                        break;
                    }
                }
                // Reclaim memory if a large line inflated the buffer.
                if line.capacity() > 64 * 1024 {
                    line.shrink_to(8 * 1024);
                }
            }
        });

        Ok(Self {
            cmd_tx,
            event_rx: rx,
        })
    }

    /// Send a command to the agent.
    pub async fn send(&mut self, cmd: &Command) -> Result<(), ClientError> {
        let mut json = serde_json::to_string(cmd)?;
        json.push('\n');
        self.cmd_tx
            .send(json)
            .await
            .map_err(|_| ClientError::Disconnected)
    }

    /// Get a cloneable command sender for use in spawned tasks.
    pub fn clone_sender(&self) -> CommandSender {
        CommandSender {
            tx: self.cmd_tx.clone(),
        }
    }

    /// Receive the next event from the agent.
    ///
    /// Returns `None` if the connection is closed.
    pub async fn recv(&mut self) -> Option<Event> {
        self.event_rx.recv().await
    }

    /// Try to receive an event without blocking.
    pub fn try_recv(&mut self) -> Option<Event> {
        self.event_rx.try_recv().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_serializes_to_json_lines() {
        let cmd = Command::Prompt {
            id: Some("p-1".into()),
            message: "hello".into(),
            streaming_behavior: None,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"type\":\"prompt\""));
        assert!(json.contains("\"message\":\"hello\""));
        assert!(json.contains("\"id\":\"p-1\""));
        // streaming_behavior should be omitted when None
        assert!(!json.contains("streamingBehavior"));
    }

    #[test]
    fn command_prompt_with_streaming_behavior() {
        let cmd = Command::Prompt {
            id: None,
            message: "hi".into(),
            streaming_behavior: Some("steer".into()),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"streamingBehavior\":\"steer\""));
    }

    #[test]
    fn command_get_state_serializes() {
        let cmd = Command::GetState {
            id: Some("gs-1".into()),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"type\":\"get_state\""));
    }

    #[test]
    fn command_abort_serializes() {
        let cmd = Command::Abort { id: None };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"type\":\"abort\""));
        assert!(!json.contains("\"id\""));
    }

    #[test]
    fn event_deserializes_agent_start() {
        let json = r#"{"type":"agent_start"}"#;
        let event: Event = serde_json::from_str(json).unwrap();
        assert!(matches!(event, Event::AgentStart));
    }

    #[test]
    fn event_deserializes_token() {
        let json = r#"{"type":"token","token":"hello"}"#;
        let event: Event = serde_json::from_str(json).unwrap();
        match event {
            Event::Token { token } => assert_eq!(token, "hello"),
            _ => panic!("expected Token event"),
        }
    }

    #[test]
    fn event_deserializes_response() {
        let json =
            r#"{"type":"response","command":"get_state","success":true,"data":{"model":"test"}}"#;
        let event: Event = serde_json::from_str(json).unwrap();
        match event {
            Event::Response {
                command, success, ..
            } => {
                assert_eq!(command, "get_state");
                assert!(success);
            }
            _ => panic!("expected Response event"),
        }
    }

    #[test]
    fn event_deserializes_tool_execution_start() {
        let json = r#"{"type":"tool_execution_start","toolCallId":"c-1","toolName":"bash","args":{"command":"ls"}}"#;
        let event: Event = serde_json::from_str(json).unwrap();
        match event {
            Event::ToolExecutionStart {
                tool_call_id,
                tool_name,
                ..
            } => {
                assert_eq!(tool_call_id, "c-1");
                assert_eq!(tool_name, "bash");
            }
            _ => panic!("expected ToolExecutionStart"),
        }
    }

    #[test]
    fn event_deserializes_tool_execution_end() {
        let json = r#"{"type":"tool_execution_end","toolCallId":"c-1","toolName":"bash","result":{"content":[]},"isError":false}"#;
        let event: Event = serde_json::from_str(json).unwrap();
        match event {
            Event::ToolExecutionEnd {
                is_error,
                tool_name,
                ..
            } => {
                assert!(!is_error);
                assert_eq!(tool_name, "bash");
            }
            _ => panic!("expected ToolExecutionEnd"),
        }
    }

    #[test]
    fn event_deserializes_subagent_state_changed() {
        let json = r#"{"type":"subagent_state_changed","subagents":[{"agentId":"reviewer","status":"running","lastTool":"bash","pid":123}]}"#;
        let event: Event = serde_json::from_str(json).unwrap();
        match event {
            Event::SubagentStateChanged { subagents } => {
                assert_eq!(subagents.len(), 1);
                assert_eq!(subagents[0].agent_id, "reviewer");
                assert_eq!(subagents[0].status, "running");
                assert_eq!(subagents[0].last_tool.as_deref(), Some("bash"));
                assert_eq!(subagents[0].pid, 123);
            }
            _ => panic!("expected SubagentStateChanged"),
        }
    }

    #[test]
    fn event_subagent_state_changed_empty() {
        let json = r#"{"type":"subagent_state_changed","subagents":[]}"#;
        let event: Event = serde_json::from_str(json).unwrap();
        match event {
            Event::SubagentStateChanged { subagents } => assert!(subagents.is_empty()),
            _ => panic!("expected SubagentStateChanged"),
        }
    }

    #[test]
    fn event_subagent_info_with_error() {
        let json = r#"{"type":"subagent_state_changed","subagents":[{"agentId":"lint","status":"error","lastError":"tool 'bash' returned error","pid":0}]}"#;
        let event: Event = serde_json::from_str(json).unwrap();
        match event {
            Event::SubagentStateChanged { subagents } => {
                assert_eq!(
                    subagents[0].last_error.as_deref(),
                    Some("tool 'bash' returned error")
                );
                assert!(subagents[0].last_tool.is_none());
            }
            _ => panic!("expected SubagentStateChanged"),
        }
    }

    #[test]
    fn command_get_subagents_serializes() {
        let cmd = Command::GetSubagents {
            id: Some("gs-1".into()),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"type\":\"get_subagents\""));
        assert!(json.contains("\"id\":\"gs-1\""));
    }

    #[test]
    fn event_unknown_type_deserializes_as_unknown() {
        let json = r#"{"type":"some_future_event","data":42}"#;
        let event: Event = serde_json::from_str(json).unwrap();
        assert!(matches!(event, Event::Unknown));
    }

    #[test]
    fn event_deserializes_agent_end() {
        let json = r#"{"type":"agent_end","messages":[{"role":"assistant","content":"hi"}]}"#;
        let event: Event = serde_json::from_str(json).unwrap();
        match event {
            Event::AgentEnd { messages } => {
                assert_eq!(messages.len(), 1);
            }
            _ => panic!("expected AgentEnd"),
        }
    }

    // ── Integration: result text extraction ─────────────────────────

    #[test]
    fn tool_end_read_result_extraction() {
        // Simulate the exact JSON the quecto server sends for a read tool result.
        let json = r#"{"type":"tool_execution_end","toolCallId":"c-1","toolName":"read","result":{"content":[{"type":"text","text":"fn main() {\n    println!(\"hello\");\n}"}]},"isError":false}"#;
        let event: Event = serde_json::from_str(json).unwrap();
        match event {
            Event::ToolExecutionEnd { result, .. } => {
                let text = extract_result_text(&result);
                assert!(
                    text.contains("fn main()"),
                    "should extract read content: {:?}",
                    text
                );
            }
            _ => panic!("expected ToolExecutionEnd"),
        }
    }

    #[test]
    fn tool_end_bash_result_extraction() {
        let json = r#"{"type":"tool_execution_end","toolCallId":"c-1","toolName":"bash","result":{"content":[{"type":"text","text":"file1.txt\nfile2.txt\nfile3.txt"}]},"isError":false}"#;
        let event: Event = serde_json::from_str(json).unwrap();
        match event {
            Event::ToolExecutionEnd { result, .. } => {
                let text = extract_result_text(&result);
                assert!(text.contains("file1.txt"), "should extract bash output");
                assert!(text.contains("file3.txt"), "should have all lines");
            }
            _ => panic!("expected ToolExecutionEnd"),
        }
    }

    #[test]
    fn tool_end_empty_content_array() {
        let json = r#"{"type":"tool_execution_end","toolCallId":"c-1","toolName":"bash","result":{"content":[]},"isError":false}"#;
        let event: Event = serde_json::from_str(json).unwrap();
        match event {
            Event::ToolExecutionEnd { result, .. } => {
                let text = extract_result_text(&result);
                assert_eq!(text, "", "empty content array → empty string");
            }
            _ => panic!("expected ToolExecutionEnd"),
        }
    }

    #[test]
    fn tool_end_edit_result_extraction() {
        let json = r#"{"type":"tool_execution_end","toolCallId":"c-1","toolName":"edit","result":{"content":[{"type":"text","text":"Applied edit to src/main.rs\n+added\n-removed"}]},"isError":false}"#;
        let event: Event = serde_json::from_str(json).unwrap();
        match event {
            Event::ToolExecutionEnd { result, .. } => {
                let text = extract_result_text(&result);
                assert!(text.contains("+added"), "should extract diff content");
                assert!(text.contains("-removed"), "should have diff lines");
            }
            _ => panic!("expected ToolExecutionEnd"),
        }
    }

    // --- Missing Command serialization tests ---

    #[test]
    fn command_steer_serializes() {
        let cmd = Command::Steer {
            id: Some("s-1".into()),
            message: "go left".into(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"type\":\"steer\""));
        assert!(json.contains("\"message\":\"go left\""));
    }

    #[test]
    fn command_follow_up_serializes() {
        let cmd = Command::FollowUp {
            id: None,
            message: "also do this".into(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"type\":\"follow_up\""));
        assert!(json.contains("also do this"));
    }

    #[test]
    fn command_get_messages_serializes() {
        let cmd = Command::GetMessages { id: None };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"type\":\"get_messages\""));
    }

    #[test]
    fn command_get_messages_tail_serializes() {
        let cmd = Command::GetMessagesTail {
            id: Some("gmt".into()),
            count: 5,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"type\":\"get_messages_tail\""));
        assert!(json.contains("\"count\":5"));
    }

    #[test]
    fn command_get_session_stats_serializes() {
        let cmd = Command::GetSessionStats {
            id: Some("stats".into()),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"type\":\"get_session_stats\""));
    }

    #[test]
    fn command_set_model_serializes() {
        let cmd = Command::SetModel {
            id: None,
            model: Some("gpt-4o".into()),
            provider: None,
            model_id: None,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"type\":\"set_model\""));
        assert!(json.contains("\"model\":\"gpt-4o\""));
        assert!(!json.contains("provider"));
        assert!(!json.contains("modelId"));
    }

    #[test]
    fn command_set_model_with_provider() {
        let cmd = Command::SetModel {
            id: None,
            model: None,
            provider: Some("anthropic".into()),
            model_id: Some("claude-sonnet-4-20250514".into()),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"provider\":\"anthropic\""));
        assert!(json.contains("\"modelId\":\"claude-sonnet-4-20250514\""));
    }

    #[test]
    fn command_clear_history_serializes() {
        let cmd = Command::ClearHistory { id: None };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"type\":\"clear_history\""));
    }

    // --- Event deserialization edge cases ---

    #[test]
    fn event_deserializes_turn_start() {
        let json = r#"{"type":"turn_start"}"#;
        let event: Event = serde_json::from_str(json).unwrap();
        assert!(matches!(event, Event::TurnStart));
    }

    #[test]
    fn event_deserializes_turn_end() {
        let json =
            r#"{"type":"turn_end","message":{"role":"assistant","content":"hi"},"toolResults":[]}"#;
        let event: Event = serde_json::from_str(json).unwrap();
        match event {
            Event::TurnEnd { message, .. } => {
                assert_eq!(message["role"], "assistant");
            }
            _ => panic!("expected TurnEnd"),
        }
    }

    #[test]
    fn event_deserializes_extensions_changed() {
        let json = r#"{"type":"extensions_changed","extensions":[]}"#;
        let event: Event = serde_json::from_str(json).unwrap();
        assert!(matches!(event, Event::ExtensionsChanged { .. }));
    }

    #[test]
    fn event_response_with_error() {
        let json = r#"{"type":"response","command":"set_model","success":false,"error":"model not found"}"#;
        let event: Event = serde_json::from_str(json).unwrap();
        match event {
            Event::Response { success, error, .. } => {
                assert!(!success);
                assert_eq!(error.as_deref(), Some("model not found"));
            }
            _ => panic!("expected Response"),
        }
    }

    // --- extract_result_text edge cases ---

    #[test]
    fn extract_result_text_no_content_key() {
        let val = serde_json::json!({"something": "else"});
        assert_eq!(extract_result_text(&val), "");
    }

    #[test]
    fn extract_result_text_non_text_type() {
        let val = serde_json::json!({"content": [{"type": "image", "data": "abc"}]});
        assert_eq!(extract_result_text(&val), "");
    }

    // --- ClientError Display ---

    #[test]
    fn client_error_display() {
        let e = ClientError::Disconnected;
        assert_eq!(e.to_string(), "disconnected from agent");
    }

    #[test]
    fn client_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "not found");
        let e = ClientError::from(io_err);
        assert!(e.to_string().contains("I/O error"));
    }

    #[test]
    fn client_error_from_json() {
        let json_err: serde_json::Error = serde_json::from_str::<Event>("bad json").unwrap_err();
        let e = ClientError::from(json_err);
        assert!(e.to_string().contains("JSON error"));
    }
}
