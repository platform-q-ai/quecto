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

// ─── Protocol types (subset matching quecto's wire format) ────────────────────

/// A command sent from the TUI to the agent.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    Prompt {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        message: String,
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
        model: String,
    },
    ClearHistory {
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
    ExtensionsChanged {
        extensions: Vec<serde_json::Value>,
    },
    /// Catch-all for unknown event types (forward-compatible).
    #[serde(other)]
    Unknown,
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

/// A UDS client connection to a quecto agent.
///
/// The client provides:
/// - `send()` to send commands to the agent
/// - `events()` to receive a stream of events via an mpsc channel
///
/// The event reader runs in a background tokio task.
pub struct Client {
    writer: tokio::io::WriteHalf<UnixStream>,
    /// Channel for receiving events from the background reader.
    event_rx: mpsc::Receiver<Event>,
}

impl Client {
    /// Connect to a quecto agent at the given socket path.
    pub async fn connect(socket_path: &Path) -> Result<Self, ClientError> {
        let stream = UnixStream::connect(socket_path).await?;
        let (read_half, write_half) = tokio::io::split(stream);

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
                                eprintln!(
                                    "quecto-tui: failed to parse agent event: {e}: {trimmed}"
                                );
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("quecto-tui: error reading from agent socket: {e}");
                        break;
                    }
                }
            }
        });

        Ok(Self {
            writer: write_half,
            event_rx: rx,
        })
    }

    /// Send a command to the agent.
    pub async fn send(&mut self, cmd: &Command) -> Result<(), ClientError> {
        let mut json = serde_json::to_string(cmd)?;
        json.push('\n');
        self.writer.write_all(json.as_bytes()).await?;
        self.writer.flush().await?;
        Ok(())
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
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"type\":\"prompt\""));
        assert!(json.contains("\"message\":\"hello\""));
        assert!(json.contains("\"id\":\"p-1\""));
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
}
