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
    ListModels {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    ListSessions {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    NewSession {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    ResumeSession {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        session: String,
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
    SetWorkflowAutomation {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(rename = "autoContinue", skip_serializing_if = "Option::is_none")]
        auto_continue: Option<bool>,
        #[serde(rename = "completionNudge", skip_serializing_if = "Option::is_none")]
        completion_nudge: Option<bool>,
    },
    ClearHistory {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    RewindTo {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(rename = "messageIndex")]
        message_index: usize,
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
        /// Identity of the emitting agent (PRD Stage B). `None` for the
        /// connected agent's own events; set to a child's id on events the
        /// parent's monitor forwards up — those must NOT clobber the parent's
        /// own workflow bar.
        #[serde(default)]
        agent_id: Option<String>,
        // Forwarded child events (PRD Stage B) are re-emitted canonically with
        // only type/agent_id/parent_id/mode/progress — no `steps`. Default these
        // so such events still parse (then the handler ignores them by agent_id)
        // instead of failing and printing raw JSON over the TUI.
        #[serde(default)]
        steps: Vec<serde_json::Value>,
        #[serde(default)]
        progress: serde_json::Value,
        #[serde(rename = "activeIssue", default)]
        active_issue: Option<serde_json::Value>,
        #[serde(default)]
        mode: Option<String>,
        #[serde(rename = "activeTemplate", default)]
        active_template: Option<serde_json::Value>,
        #[serde(rename = "availableTemplates", default)]
        available_templates: Option<Vec<serde_json::Value>>,
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
    /// Spawning agent's id, for reconstructing the unit tree (PRD Stage B).
    #[serde(default)]
    pub parent_id: Option<String>,
    /// Latest workflow snapshot for this subagent, if any (PRD Stage B).
    #[serde(default)]
    pub workflow: Option<SubagentWorkflow>,
}

/// Workflow snapshot mirror carried on a subagent entry (PRD Stage B).
/// Field names match the server's snake_case `WorkflowSnapshot` serialization.
#[derive(Debug, Clone, Deserialize)]
pub struct SubagentWorkflow {
    pub mode: String,
    pub steps_completed: u32,
    pub steps_total: u32,
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

impl Command {
    /// Non-sensitive command kind for user-facing diagnostics.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Prompt { .. } => "prompt",
            Self::Steer { .. } => "steer",
            Self::FollowUp { .. } => "follow_up",
            Self::Abort { .. } => "abort",
            Self::GetState { .. } => "get_state",
            Self::GetMessages { .. } => "get_messages",
            Self::GetMessagesTail { .. } => "get_messages_tail",
            Self::GetSessionStats { .. } => "get_session_stats",
            Self::ListModels { .. } => "list_models",
            Self::ListSessions { .. } => "list_sessions",
            Self::NewSession { .. } => "new_session",
            Self::ResumeSession { .. } => "resume_session",
            Self::SetModel { .. } => "set_model",
            Self::SetWorkflowAutomation { .. } => "set_workflow_automation",
            Self::ClearHistory { .. } => "clear_history",
            Self::RewindTo { .. } => "rewind_to",
            Self::GetSubagents { .. } => "get_subagents",
        }
    }
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
                            Err(_e) => {
                                // Drop unparseable events silently. The TUI owns
                                // the terminal, so printing to stderr here paints
                                // the raw event (e.g. a forwarded workflow_state's
                                // JSON) over the UI — the "percent N" leak. Known
                                // event types parse via serde defaults; truly
                                // malformed lines are simply ignored.
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

    #[cfg(test)]
    pub fn disconnected_for_tests() -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel::<String>(1);
        drop(cmd_rx);
        let (_event_tx, event_rx) = mpsc::channel::<Event>(1);
        Self { cmd_tx, event_rx }
    }
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
