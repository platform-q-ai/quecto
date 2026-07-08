//! UDS client — connects to a quecto agent over a Unix domain socket.
//!
//! Sends JSON-lines commands and receives JSON-lines events. The client
//! is async (tokio) and designed to run in a background task, feeding
//! events to the TUI's main render loop.

use quecto_line_io::read_bounded_line_into;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

/// Maximum line size from the agent — derived from the shared protocol cap
/// (`quecto_line_io::PROTOCOL_LINE_CAP_BYTES`, 1 MiB) so the harness emitter
/// and this reader can never disagree (#1047 review).
///
/// Public so out-of-crate tests (the harness BDD suite) can build boundary
/// frames against the real cap instead of a duplicated literal.
pub const MAX_LINE_BYTES: usize = quecto_line_io::PROTOCOL_LINE_CAP_BYTES;

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
        /// When set, fetch this spawned sub-agent's message tail instead of the
        /// connected agent's own history (#795).
        #[serde(rename = "agent_id", skip_serializing_if = "Option::is_none")]
        agent_id: Option<String>,
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
    AgentEnd,
    Token {
        token: String,
    },
    TurnStart,
    TurnEnd {
        message: serde_json::Value,
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
    /// A passive one-line completion note from a spawned sub-agent (#816). The
    /// kernel emits this when a child completes/idles/errors while the parent is
    /// idle; the TUI renders it as a single non-interactive status line.
    #[serde(rename_all = "camelCase")]
    SubagentNotification {
        agent_id: String,
        #[serde(default)]
        sequence: u64,
        message: String,
    },
    /// A sub-agent completed a turn; carries that turn's appended messages
    /// (assistant + tool results), re-stamped by the parent monitor with the
    /// child's id. Lets the inspector stream output turn-by-turn (#797).
    SubagentMessagesAppended {
        agent_id: String,
        messages: Vec<serde_json::Value>,
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
    /// Path to this sub-agent's own UDS socket, used to open a direct
    /// connect-on-select connection to its live stream (#800). `None` when the
    /// kernel did not surface it (older servers / non-local agents).
    #[serde(default)]
    pub socket_path: Option<String>,
    /// Spawning agent's id, for reconstructing the unit tree (PRD Stage B).
    #[serde(default)]
    pub parent_id: Option<String>,
    /// Latest workflow snapshot for this subagent, if any (PRD Stage B).
    #[serde(default)]
    pub workflow: Option<SubagentWorkflow>,
    /// Whether this sub-agent was spawned read-only (`write` + `edit` disabled).
    /// Drives the observer marker in the left panel (#966). Defaults to `false`
    /// for older kernels that did not surface the field.
    #[serde(default)]
    pub read_only: bool,
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

/// Serialize a command to its JSON-lines wire form (JSON + trailing newline).
///
/// Both [`CommandSender::send`] and [`Client::send`] write the same framed wire
/// format, so the serialize-and-newline rule lives here in one place.
fn serialize_command(cmd: &Command) -> Result<String, ClientError> {
    let mut json = serde_json::to_string(cmd)?;
    json.push('\n');
    Ok(json)
}

impl CommandSender {
    /// Send a command to the agent.
    pub async fn send(&mut self, cmd: &Command) -> Result<(), ClientError> {
        self.tx
            .send(serialize_command(cmd)?)
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
    /// Count of event lines the reader dropped for exceeding
    /// [`MAX_LINE_BYTES`], so the UI can surface the loss instead of the
    /// session silently appearing frozen (#1047).
    dropped_oversized: std::sync::Arc<std::sync::atomic::AtomicU64>,
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
        let dropped_oversized = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let dropped_counter = std::sync::Arc::clone(&dropped_oversized);

        // Spawn background event reader
        tokio::spawn(async move {
            let mut reader = BufReader::new(read_half);
            let mut line = Vec::new();
            loop {
                match read_bounded_line_into(&mut reader, &mut line, MAX_LINE_BYTES).await {
                    Ok(None) => break, // EOF — agent closed the connection
                    Ok(Some(read)) => {
                        // Enforce max line size while framing so malicious/buggy agents
                        // cannot inflate this buffer beyond the protocol cap.
                        if read.truncated
                            || (line.len() >= MAX_LINE_BYTES && !line.ends_with(b"\n"))
                        {
                            // Drop oversized lines without printing (the TUI
                            // owns the terminal, so stderr would smear
                            // diagnostics over the UI) — but COUNT the drop so
                            // the UI can surface the loss instead of the
                            // session silently appearing frozen (#1047).
                            dropped_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            continue;
                        }
                        let trimmed = match std::str::from_utf8(&line) {
                            Ok(value) => value.trim(),
                            Err(_) => continue,
                        };
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
                    Err(_e) => {
                        // Stop reading silently on a socket error; printing to
                        // stderr here would paint over the TUI (see no-stderr
                        // policy above). The closed channel will surface the
                        // disconnect to the UI on the next send.
                        break;
                    }
                }
            }
        });

        Ok(Self {
            cmd_tx,
            event_rx: rx,
            dropped_oversized,
        })
    }

    /// How many event lines the reader has dropped for exceeding
    /// [`MAX_LINE_BYTES`] (#1047). The UI polls this to surface the drop.
    pub fn dropped_oversized_events(&self) -> u64 {
        self.dropped_oversized
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Test-only: simulate the reader recording `n` oversized-line drops, so
    /// UI-surfacing tests don't need to stream a >1 MiB frame.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn record_dropped_oversized_for_tests(&self, n: u64) {
        self.dropped_oversized
            .fetch_add(n, std::sync::atomic::Ordering::Relaxed);
    }

    /// Send a command to the agent.
    pub async fn send(&mut self, cmd: &Command) -> Result<(), ClientError> {
        self.cmd_tx
            .send(serialize_command(cmd)?)
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

    /// Try to receive an event without blocking (tests only).
    #[cfg(test)]
    pub fn try_recv(&mut self) -> Option<Event> {
        self.event_rx.try_recv().ok()
    }

    #[cfg(any(test, feature = "test-harness"))]
    pub fn disconnected_for_tests() -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel::<String>(1);
        drop(cmd_rx);
        let (_event_tx, event_rx) = mpsc::channel::<Event>(1);
        Self {
            cmd_tx,
            event_rx,
            dropped_oversized: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }
}

#[cfg(test)]
#[path = "client_defence_tests.rs"]
mod client_defence_tests;

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
