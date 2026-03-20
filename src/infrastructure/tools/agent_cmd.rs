// Agent command tool: native UDS interaction with spawned subagents (#421).
//
// Connects to child agent UDS sockets directly from Rust — no ncat, no socat,
// no bash intermediary.  Uses the existing JSON-lines protocol from
// `src/interface/cli/protocol.rs`.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};

// Re-export shared types for external consumers.
pub use super::subagent_registry::{
    SubagentEntry, SubagentRegistry, new_registry, validate_agent_id_format,
};

/// Supported commands for interacting with a subagent.
const SUPPORTED_COMMANDS: &[&str] = &[
    "prompt",
    "get_state",
    "get_messages_tail",
    "steer",
    "abort",
    "get_session_stats",
];

/// Timeout for reading a response from a subagent UDS socket.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(300);

/// Tool that sends UDS commands to spawned subagents.
///
/// Looks up the socket path from a shared [`SubagentRegistry`], connects,
/// sends the JSON-lines command, reads the response, and returns it as a
/// structured [`ToolResult`].
#[derive(Debug, Clone)]
pub struct AgentCmdTool {
    /// Shared registry populated by [`super::spawn::SpawnTool`].
    registry: SubagentRegistry,
}

impl AgentCmdTool {
    /// Create a new `AgentCmdTool` backed by the given registry.
    pub fn new(registry: SubagentRegistry) -> Self {
        Self { registry }
    }

    /// Create a new empty registry (convenience for tests and wiring).
    pub fn new_registry() -> SubagentRegistry {
        new_registry()
    }

    /// Parse arguments and build the JSON command to send.
    fn parse_and_build(&self, arguments: &str) -> Result<(String, String), String> {
        let args: serde_json::Value =
            serde_json::from_str(arguments).map_err(|e| format!("invalid JSON: {e}"))?;

        let agent_id = args
            .get("agent_id")
            .and_then(|v| v.as_str())
            .ok_or("missing required field: agent_id")?
            .to_string();

        // Validate agent_id format (same rules as spawn).
        validate_agent_id_format(&agent_id)?;

        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or("missing required field: command")?
            .to_string();

        if !SUPPORTED_COMMANDS.contains(&command.as_str()) {
            return Err(format!(
                "unsupported command '{}'; supported: {}",
                command,
                SUPPORTED_COMMANDS.join(", ")
            ));
        }

        // Build the JSON-lines command.
        let json_cmd = match command.as_str() {
            "prompt" => {
                let message = args
                    .get("message")
                    .and_then(|v| v.as_str())
                    .ok_or("prompt command requires a message field")?;
                serde_json::json!({"type": "prompt", "message": message})
            }
            "steer" => {
                let message = args
                    .get("message")
                    .and_then(|v| v.as_str())
                    .ok_or("steer command requires a message field")?;
                serde_json::json!({"type": "steer", "message": message})
            }
            "get_state" => serde_json::json!({"type": "get_state"}),
            "abort" => serde_json::json!({"type": "abort"}),
            "get_session_stats" => serde_json::json!({"type": "get_session_stats"}),
            "get_messages_tail" => {
                let count = args.get("count").and_then(|v| v.as_u64()).unwrap_or(1);
                serde_json::json!({"type": "get_messages_tail", "count": count})
            }
            _ => unreachable!(), // Covered by SUPPORTED_COMMANDS check above.
        };

        Ok((agent_id, json_cmd.to_string()))
    }

    /// Look up the socket path for an agent ID.
    fn lookup_socket(&self, agent_id: &str) -> Result<std::path::PathBuf, String> {
        let entries = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        entries
            .get(agent_id)
            .map(|e| e.socket_path.clone())
            .ok_or_else(|| format!("subagent '{}' not found in registry", agent_id))
    }
}

/// Send a JSON-lines command to a UDS socket and read the first response line.
///
/// Each call opens a new connection, sends the command, and reads one response
/// line. The connection is closed after each call.
// TODO: consider connection pooling for frequent polling patterns.
async fn send_uds_command(
    socket_path: &std::path::Path,
    command: &str,
) -> Result<String, DomainError> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let stream = tokio::net::UnixStream::connect(socket_path)
        .await
        .map_err(|e| {
            DomainError::Tool(format!(
                "connect to subagent at {} failed: {e}",
                socket_path.display()
            ))
        })?;

    let (reader, mut writer) = tokio::io::split(stream);

    writer
        .write_all(command.as_bytes())
        .await
        .map_err(|e| DomainError::Tool(format!("write to subagent failed: {e}")))?;
    writer
        .write_all(b"\n")
        .await
        .map_err(|e| DomainError::Tool(format!("write to subagent failed: {e}")))?;
    writer
        .shutdown()
        .await
        .map_err(|e| DomainError::Tool(format!("shutdown write half failed: {e}")))?;

    let mut lines = BufReader::new(reader).lines();

    // Read with timeout to avoid blocking indefinitely if the child hangs.
    let response = tokio::time::timeout(RESPONSE_TIMEOUT, lines.next_line())
        .await
        .map_err(|_| DomainError::Tool("subagent response timed out (300s)".into()))?
        .map_err(|e| DomainError::Tool(format!("read from subagent failed: {e}")))?
        .unwrap_or_default();

    Ok(response)
}

impl Tool for AgentCmdTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "agent_cmd".into(),
            description: "Send a command to a spawned subagent. \
                Supported commands: prompt, get_state, get_messages_tail, \
                steer, abort, get_session_stats."
                .into(),
            parameters_schema: r#"{"type":"object","properties":{"agent_id":{"type":"string","description":"ID of the spawned subagent"},"command":{"type":"string","enum":["prompt","get_state","get_messages_tail","steer","abort","get_session_stats"],"description":"Command to send"},"message":{"type":"string","description":"Message for prompt/steer commands"},"count":{"type":"integer","description":"Number of messages for get_messages_tail (default: 1)"}},"required":["agent_id","command"]}"#.into(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let args = arguments.to_string();
        Box::pin(async move {
            // Parse and validate arguments.
            let (agent_id, json_cmd) = match self.parse_and_build(&args) {
                Ok(pair) => pair,
                Err(e) => {
                    return Ok(ToolResult {
                        content: format!("agent_cmd error: {e}"),
                        is_error: true,
                        image_blocks: vec![],
                    });
                }
            };

            // Look up the socket.
            let socket_path = match self.lookup_socket(&agent_id) {
                Ok(p) => p,
                Err(e) => {
                    return Ok(ToolResult {
                        content: format!("agent_cmd error: {e}"),
                        is_error: true,
                        image_blocks: vec![],
                    });
                }
            };

            // Send the command via UDS.
            match send_uds_command(&socket_path, &json_cmd).await {
                Ok(response) => Ok(ToolResult {
                    content: response,
                    is_error: false,
                    image_blocks: vec![],
                }),
                Err(e) => Ok(ToolResult {
                    content: format!("agent_cmd error: {e}"),
                    is_error: true,
                    image_blocks: vec![],
                }),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn empty_tool() -> AgentCmdTool {
        AgentCmdTool::new(new_registry())
    }

    #[test]
    fn test_definition_name() {
        let tool = empty_tool();
        assert_eq!(tool.definition().name, "agent_cmd");
    }

    #[test]
    fn test_definition_requires_agent_id_and_command() {
        let tool = empty_tool();
        let def = tool.definition();
        let schema: serde_json::Value = serde_json::from_str(&def.parameters_schema).unwrap();
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("agent_id")));
        assert!(required.iter().any(|v| v.as_str() == Some("command")));
    }

    #[test]
    fn test_parse_missing_agent_id() {
        let tool = empty_tool();
        let result = tool.parse_and_build(r#"{"command":"get_state"}"#);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("agent_id"));
    }

    #[test]
    fn test_parse_missing_command() {
        let tool = empty_tool();
        let result = tool.parse_and_build(r#"{"agent_id":"w1"}"#);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("command"));
    }

    #[test]
    fn test_parse_invalid_json() {
        let tool = empty_tool();
        let result = tool.parse_and_build("not json");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid JSON"));
    }

    #[test]
    fn test_parse_unsupported_command() {
        let tool = empty_tool();
        let result = tool.parse_and_build(r#"{"agent_id":"w1","command":"delete_all"}"#);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unsupported command"));
    }

    #[test]
    fn test_parse_invalid_agent_id_format() {
        let tool = empty_tool();
        let result = tool.parse_and_build(r#"{"agent_id":"bad id!","command":"get_state"}"#);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("[a-zA-Z0-9_-]"));
    }

    #[test]
    fn test_parse_get_state_builds_json() {
        let tool = empty_tool();
        let (agent_id, cmd) = tool
            .parse_and_build(r#"{"agent_id":"w1","command":"get_state"}"#)
            .unwrap();
        assert_eq!(agent_id, "w1");
        let parsed: serde_json::Value = serde_json::from_str(&cmd).unwrap();
        assert_eq!(parsed["type"], "get_state");
    }

    #[test]
    fn test_parse_prompt_requires_message() {
        let tool = empty_tool();
        let result = tool.parse_and_build(r#"{"agent_id":"w1","command":"prompt"}"#);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("message"));
    }

    #[test]
    fn test_parse_prompt_with_message() {
        let tool = empty_tool();
        let (_, cmd) = tool
            .parse_and_build(r#"{"agent_id":"w1","command":"prompt","message":"hello"}"#)
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&cmd).unwrap();
        assert_eq!(parsed["type"], "prompt");
        assert_eq!(parsed["message"], "hello");
    }

    #[test]
    fn test_parse_steer_requires_message() {
        let tool = empty_tool();
        let result = tool.parse_and_build(r#"{"agent_id":"w1","command":"steer"}"#);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("message"));
    }

    #[test]
    fn test_parse_get_messages_tail_default_count() {
        let tool = empty_tool();
        let (_, cmd) = tool
            .parse_and_build(r#"{"agent_id":"w1","command":"get_messages_tail"}"#)
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&cmd).unwrap();
        assert_eq!(parsed["type"], "get_messages_tail");
        assert_eq!(parsed["count"], 1);
    }

    #[test]
    fn test_parse_get_messages_tail_custom_count() {
        let tool = empty_tool();
        let (_, cmd) = tool
            .parse_and_build(r#"{"agent_id":"w1","command":"get_messages_tail","count":5}"#)
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&cmd).unwrap();
        assert_eq!(parsed["count"], 5);
    }

    #[test]
    fn test_parse_abort() {
        let tool = empty_tool();
        let (_, cmd) = tool
            .parse_and_build(r#"{"agent_id":"w1","command":"abort"}"#)
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&cmd).unwrap();
        assert_eq!(parsed["type"], "abort");
    }

    #[test]
    fn test_parse_get_session_stats() {
        let tool = empty_tool();
        let (_, cmd) = tool
            .parse_and_build(r#"{"agent_id":"w1","command":"get_session_stats"}"#)
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&cmd).unwrap();
        assert_eq!(parsed["type"], "get_session_stats");
    }

    #[test]
    fn test_lookup_unknown_agent() {
        let tool = empty_tool();
        let result = tool.lookup_socket("nonexistent");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_lookup_known_agent() {
        let registry = new_registry();
        registry.lock().unwrap().insert(
            "w1".to_string(),
            SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0),
        );
        let tool = AgentCmdTool::new(registry);
        let path = tool.lookup_socket("w1").unwrap();
        assert_eq!(path, PathBuf::from("/tmp/test.sock"));
    }

    #[tokio::test]
    async fn test_execute_unknown_agent_returns_error() {
        let tool = empty_tool();
        let result = tool
            .execute(r#"{"agent_id":"nonexistent","command":"get_state"}"#)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("not found"));
    }

    #[tokio::test]
    async fn test_execute_invalid_json_returns_error() {
        let tool = empty_tool();
        let result = tool.execute("not json").await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("invalid JSON"));
    }

    #[tokio::test]
    async fn test_execute_missing_fields_returns_error() {
        let tool = empty_tool();
        let result = tool.execute(r#"{"agent_id":"w1"}"#).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("command"));
    }

    #[tokio::test]
    async fn test_execute_invalid_agent_id_format_returns_error() {
        let tool = empty_tool();
        let result = tool
            .execute(r#"{"agent_id":"bad id!","command":"get_state"}"#)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("[a-zA-Z0-9_-]"));
    }
}
