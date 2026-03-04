/// RPC protocol types for `quecto agent --mode rpc`.
///
/// JSON-lines protocol over stdin/stdout.  One JSON object per line.
/// All commands carry an optional `id` field for request/response correlation.
use serde::{Deserialize, Serialize};

// ─── Commands (stdin) ────────────────────────────────────────────────────────

/// A command received from stdin.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RpcCommand {
    /// Send a user message to the agent.
    Prompt {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        message: String,
        /// Required when the agent is currently running.
        #[serde(rename = "streamingBehavior", skip_serializing_if = "Option::is_none")]
        streaming_behavior: Option<StreamingBehavior>,
    },
    /// Interrupt after the current tool, then deliver this message.
    Steer {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        message: String,
    },
    /// Deliver this message when the agent finishes.
    FollowUp {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        message: String,
    },
    /// Cancel the current agent run.
    Abort {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Return current session state.
    GetState {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Return the full conversation history.
    GetMessages {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Return the last `count` messages from the conversation history.
    GetMessagesTail {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        count: usize,
    },
    /// Return token usage and cost statistics.
    GetSessionStats {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Switch the active model at runtime.
    ///
    /// Accepts either:
    /// - legacy `{ "model": "provider/modelId" }`, or
    /// - Pi-compatible `{ "provider": "...", "modelId": "..." }`.
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
}

impl RpcCommand {
    /// Return the optional correlation id.
    pub fn id(&self) -> Option<&str> {
        match self {
            Self::Prompt { id, .. } => id.as_deref(),
            Self::Steer { id, .. } => id.as_deref(),
            Self::FollowUp { id, .. } => id.as_deref(),
            Self::Abort { id } => id.as_deref(),
            Self::GetState { id } => id.as_deref(),
            Self::GetMessages { id } => id.as_deref(),
            Self::GetMessagesTail { id, .. } => id.as_deref(),
            Self::GetSessionStats { id } => id.as_deref(),
            Self::SetModel { id, .. } => id.as_deref(),
        }
    }

    /// Return the command name string for use in responses.
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Prompt { .. } => "prompt",
            Self::Steer { .. } => "steer",
            Self::FollowUp { .. } => "follow_up",
            Self::Abort { .. } => "abort",
            Self::GetState { .. } => "get_state",
            Self::GetMessages { .. } => "get_messages",
            Self::GetMessagesTail { .. } => "get_messages_tail",
            Self::GetSessionStats { .. } => "get_session_stats",
            Self::SetModel { .. } => "set_model",
        }
    }
}

/// How to handle a `prompt` command when the agent is already running.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum StreamingBehavior {
    /// Interrupt after the current tool; deliver the message next.
    Steer,
    /// Wait until the agent finishes; then deliver the message.
    FollowUp,
}

// ─── Events (stdout) ─────────────────────────────────────────────────────────

/// An event emitted to stdout.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RpcEvent {
    /// Agent begins processing a prompt.
    AgentStart,
    /// Agent finished processing.  Contains messages from this run as JSON values.
    AgentEnd { messages: Vec<serde_json::Value> },
    /// A new LLM call begins.
    TurnStart,
    /// LLM call completed.
    TurnEnd {
        message: TurnMessage,
        #[serde(rename = "toolResults")]
        tool_results: Vec<ToolResultEvent>,
    },
    /// A tool began executing.
    ToolExecutionStart {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        args: serde_json::Value,
    },
    /// A tool finished executing.
    ToolExecutionEnd {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        result: ToolResultContent,
        #[serde(rename = "isError")]
        is_error: bool,
    },
    /// Response to a command.
    Response {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        command: String,
        success: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TurnUsage>,
    #[serde(rename = "stopReason", skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnUsage {
    pub input: u32,
    pub output: u32,
    pub total: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultEvent {
    #[serde(rename = "toolCallId")]
    pub tool_call_id: String,
    pub content: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultContent {
    pub content: Vec<serde_json::Value>,
}

// ─── Response helpers ────────────────────────────────────────────────────────

impl RpcEvent {
    /// Build a success response.
    pub fn ok(id: Option<&str>, command: &str, data: Option<serde_json::Value>) -> Self {
        Self::Response {
            id: id.map(str::to_owned),
            command: command.to_owned(),
            success: true,
            data,
            error: None,
        }
    }

    /// Build an error response.
    pub fn err(id: Option<&str>, command: &str, message: impl Into<String>) -> Self {
        Self::Response {
            id: id.map(str::to_owned),
            command: command.to_owned(),
            success: false,
            data: None,
            error: Some(message.into()),
        }
    }

    /// Serialize the event to a JSON line (no trailing newline).
    pub fn to_json_line(&self) -> String {
        serde_json::to_string(self).expect("RpcEvent is always serializable")
    }
}

// ─── Session state snapshot ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionState {
    pub model: String,
    pub is_streaming: bool,
    pub session_key: String,
    pub message_count: usize,
    pub pending_message_count: usize,
}

// ─── Session statistics ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStats {
    pub session_key: String,
    pub user_messages: usize,
    pub assistant_messages: usize,
    pub tool_calls: usize,
    pub tool_results: usize,
    pub total_messages: usize,
    pub tokens: TokenStats,
    pub cost: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TokenStats {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub total: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── RpcCommand deserialization ──────────────────────────────────────────

    #[test]
    fn test_parse_prompt_command() {
        let json = r#"{"type":"prompt","message":"hello world"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        match cmd {
            RpcCommand::Prompt {
                message,
                id,
                streaming_behavior,
            } => {
                assert_eq!(message, "hello world");
                assert!(id.is_none());
                assert!(streaming_behavior.is_none());
            }
            _ => panic!("expected Prompt"),
        }
    }

    #[test]
    fn test_parse_prompt_with_id() {
        let json = r#"{"type":"prompt","id":"req-1","message":"hello"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.id(), Some("req-1"));
    }

    #[test]
    fn test_parse_prompt_with_steer_behavior() {
        let json = r#"{"type":"prompt","message":"hi","streamingBehavior":"steer"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        match cmd {
            RpcCommand::Prompt {
                streaming_behavior, ..
            } => {
                assert_eq!(streaming_behavior, Some(StreamingBehavior::Steer));
            }
            _ => panic!("expected Prompt"),
        }
    }

    #[test]
    fn test_parse_prompt_with_follow_up_behavior() {
        let json = r#"{"type":"prompt","message":"hi","streamingBehavior":"followUp"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        match cmd {
            RpcCommand::Prompt {
                streaming_behavior, ..
            } => {
                assert_eq!(streaming_behavior, Some(StreamingBehavior::FollowUp));
            }
            _ => panic!("expected Prompt"),
        }
    }

    #[test]
    fn test_parse_steer_command() {
        let json = r#"{"type":"steer","message":"change direction"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        match cmd {
            RpcCommand::Steer { message, .. } => assert_eq!(message, "change direction"),
            _ => panic!("expected Steer"),
        }
    }

    #[test]
    fn test_parse_follow_up_command() {
        let json = r#"{"type":"follow_up","message":"also do this"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        match cmd {
            RpcCommand::FollowUp { message, .. } => assert_eq!(message, "also do this"),
            _ => panic!("expected FollowUp"),
        }
    }

    #[test]
    fn test_parse_abort_command() {
        let json = r#"{"type":"abort"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        matches!(cmd, RpcCommand::Abort { .. });
    }

    #[test]
    fn test_parse_get_state_command() {
        let json = r#"{"type":"get_state","id":"gs-1"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.id(), Some("gs-1"));
        assert_eq!(cmd.type_name(), "get_state");
    }

    #[test]
    fn test_parse_get_messages_command() {
        let json = r#"{"type":"get_messages"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.type_name(), "get_messages");
    }

    #[test]
    fn test_parse_get_messages_tail_command() {
        let json = r#"{"type":"get_messages_tail","count":5}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        match cmd {
            RpcCommand::GetMessagesTail { id, count } => {
                assert!(id.is_none());
                assert_eq!(count, 5);
            }
            _ => panic!("expected GetMessagesTail"),
        }
    }

    #[test]
    fn test_parse_get_messages_tail_with_id() {
        let json = r#"{"type":"get_messages_tail","id":"gmt-1","count":10}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.id(), Some("gmt-1"));
        assert_eq!(cmd.type_name(), "get_messages_tail");
    }

    #[test]
    fn test_parse_get_messages_tail_count_zero() {
        let json = r#"{"type":"get_messages_tail","count":0}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        match cmd {
            RpcCommand::GetMessagesTail { count, .. } => assert_eq!(count, 0),
            _ => panic!("expected GetMessagesTail"),
        }
    }

    #[test]
    fn test_parse_get_session_stats_command() {
        let json = r#"{"type":"get_session_stats"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.type_name(), "get_session_stats");
    }

    #[test]
    fn test_parse_set_model_command() {
        let json = r#"{"type":"set_model","model":"gpt-5-mini"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        match cmd {
            RpcCommand::SetModel {
                model,
                provider,
                model_id,
                ..
            } => {
                assert_eq!(model.as_deref(), Some("gpt-5-mini"));
                assert!(provider.is_none());
                assert!(model_id.is_none());
            }
            _ => panic!("expected SetModel"),
        }
    }

    #[test]
    fn test_parse_set_model_provider_and_model_id_command() {
        let json = r#"{"type":"set_model","provider":"openai-codex","modelId":"gpt-5.3-codex"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        match cmd {
            RpcCommand::SetModel {
                model,
                provider,
                model_id,
                ..
            } => {
                assert!(model.is_none());
                assert_eq!(provider.as_deref(), Some("openai-codex"));
                assert_eq!(model_id.as_deref(), Some("gpt-5.3-codex"));
            }
            _ => panic!("expected SetModel"),
        }
    }

    #[test]
    fn test_malformed_json_fails() {
        let result: Result<RpcCommand, _> = serde_json::from_str("not json{");
        assert!(result.is_err());
    }

    #[test]
    fn test_unknown_type_fails() {
        let result: Result<RpcCommand, _> = serde_json::from_str(r#"{"type":"unknown_command"}"#);
        assert!(result.is_err());
    }

    // ─── RpcEvent serialization ──────────────────────────────────────────────

    #[test]
    fn test_agent_start_event_serializes() {
        let event = RpcEvent::AgentStart;
        let json = event.to_json_line();
        assert!(json.contains("\"type\":\"agent_start\""));
    }

    #[test]
    fn test_agent_end_event_serializes() {
        let event = RpcEvent::AgentEnd { messages: vec![] };
        let json = event.to_json_line();
        assert!(json.contains("\"type\":\"agent_end\""));
        assert!(json.contains("\"messages\""));
    }

    #[test]
    fn test_turn_start_event_serializes() {
        let event = RpcEvent::TurnStart;
        let json = event.to_json_line();
        assert!(json.contains("\"type\":\"turn_start\""));
    }

    #[test]
    fn test_tool_execution_start_event_serializes() {
        let event = RpcEvent::ToolExecutionStart {
            tool_call_id: "call-1".to_string(),
            tool_name: "bash".to_string(),
            args: serde_json::json!({"command": "echo hi"}),
        };
        let json = event.to_json_line();
        assert!(json.contains("\"type\":\"tool_execution_start\""));
        assert!(json.contains("\"toolName\":\"bash\""));
        assert!(json.contains("\"toolCallId\":\"call-1\""));
    }

    #[test]
    fn test_tool_execution_end_event_serializes() {
        let event = RpcEvent::ToolExecutionEnd {
            tool_call_id: "call-1".to_string(),
            tool_name: "bash".to_string(),
            result: ToolResultContent {
                content: vec![serde_json::json!({"type":"text","text":"hi"})],
            },
            is_error: false,
        };
        let json = event.to_json_line();
        assert!(json.contains("\"type\":\"tool_execution_end\""));
        assert!(json.contains("\"isError\":false"));
    }

    #[test]
    fn test_response_ok_event_serializes() {
        let event = RpcEvent::ok(Some("req-1"), "prompt", None);
        let json = event.to_json_line();
        assert!(json.contains("\"type\":\"response\""));
        assert!(json.contains("\"command\":\"prompt\""));
        assert!(json.contains("\"success\":true"));
        assert!(json.contains("\"id\":\"req-1\""));
    }

    #[test]
    fn test_response_err_event_serializes() {
        let event = RpcEvent::err(None, "prompt", "agent already running");
        let json = event.to_json_line();
        assert!(json.contains("\"success\":false"));
        assert!(json.contains("\"error\":\"agent already running\""));
        // id field should be absent when None
        assert!(!json.contains("\"id\""));
    }

    #[test]
    fn test_response_without_id_omits_id_field() {
        let event = RpcEvent::ok(None, "abort", None);
        let json = event.to_json_line();
        assert!(!json.contains("\"id\""));
    }

    // ─── SessionState / SessionStats ────────────────────────────────────────

    #[test]
    fn test_session_state_serializes() {
        let state = SessionState {
            model: "gpt-5".to_string(),
            is_streaming: false,
            session_key: "cli:test".to_string(),
            message_count: 4,
            pending_message_count: 0,
        };
        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains("\"isStreaming\":false"));
        assert!(json.contains("\"sessionKey\":\"cli:test\""));
        assert!(json.contains("\"messageCount\":4"));
    }

    #[test]
    fn test_session_stats_serializes() {
        let stats = SessionStats {
            session_key: "cli:test".to_string(),
            user_messages: 2,
            assistant_messages: 2,
            tool_calls: 3,
            tool_results: 3,
            total_messages: 10,
            tokens: TokenStats {
                input: 1000,
                output: 200,
                cache_read: 800,
                cache_write: 100,
                total: 2100,
            },
            cost: 0.42,
        };
        let json = serde_json::to_string(&stats).unwrap();
        assert!(json.contains("\"userMessages\":2"));
        assert!(json.contains("\"totalMessages\":10"));
        assert!(json.contains("\"tokens\""));
    }

    // ─── RpcCommand::id() / type_name() ─────────────────────────────────────

    #[test]
    fn test_command_type_names() {
        assert_eq!(RpcCommand::Abort { id: None }.type_name(), "abort");
        assert_eq!(RpcCommand::GetState { id: None }.type_name(), "get_state");
        assert_eq!(
            RpcCommand::GetMessages { id: None }.type_name(),
            "get_messages"
        );
        assert_eq!(
            RpcCommand::GetMessagesTail { id: None, count: 5 }.type_name(),
            "get_messages_tail"
        );
        assert_eq!(
            RpcCommand::GetSessionStats { id: None }.type_name(),
            "get_session_stats"
        );
        assert_eq!(
            RpcCommand::SetModel {
                id: None,
                model: Some("m".into()),
                provider: None,
                model_id: None,
            }
            .type_name(),
            "set_model"
        );
        assert_eq!(
            RpcCommand::FollowUp {
                id: None,
                message: "m".into()
            }
            .type_name(),
            "follow_up"
        );
        assert_eq!(
            RpcCommand::Steer {
                id: None,
                message: "m".into()
            }
            .type_name(),
            "steer"
        );
    }
}

#[cfg(test)]
#[path = "rpc_shape_tests.rs"]
mod shape_tests;
