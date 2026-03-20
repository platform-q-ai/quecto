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
    "steer",
    "follow_up",
    "abort",
    "kill",
    "get_state",
    "get_messages",
    "get_messages_tail",
    "get_session_stats",
    "get_subagents",
    "get_extensions",
    "set_model",
    "clear_history",
    "reload_extensions",
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
            "follow_up" => {
                let message = args
                    .get("message")
                    .and_then(|v| v.as_str())
                    .ok_or("follow_up command requires a message field")?;
                serde_json::json!({"type": "follow_up", "message": message})
            }
            "get_state" => serde_json::json!({"type": "get_state"}),
            "get_messages" => serde_json::json!({"type": "get_messages"}),
            "abort" => serde_json::json!({"type": "abort"}),
            "get_session_stats" => serde_json::json!({"type": "get_session_stats"}),
            "get_messages_tail" => {
                let count = args.get("count").and_then(|v| v.as_u64()).unwrap_or(1);
                serde_json::json!({"type": "get_messages_tail", "count": count})
            }
            "set_model" => {
                let model = args
                    .get("model")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty());
                let provider = args
                    .get("provider")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty());
                let model_id = args
                    .get("model_id")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty());
                match (model, provider, model_id) {
                    (Some(m), _, _) => serde_json::json!({"type": "set_model", "model": m}),
                    (None, Some(p), Some(mid)) => {
                        serde_json::json!({"type": "set_model", "provider": p, "modelId": mid})
                    }
                    (None, Some(_), None) => {
                        return Err("set_model: provider requires model_id".to_string());
                    }
                    (None, None, Some(_)) => {
                        return Err("set_model: model_id requires provider".to_string());
                    }
                    _ => return Err("set_model requires model, or provider + model_id".to_string()),
                }
            }
            "clear_history" => serde_json::json!({"type": "clear_history"}),
            "get_subagents" => serde_json::json!({"type": "get_subagents"}),
            "get_extensions" => serde_json::json!({"type": "get_extensions"}),
            "reload_extensions" => serde_json::json!({"type": "reload_extensions"}),
            "kill" => unreachable!("kill is handled by try_local_command before parse_and_build"),
            _ => unreachable!(), // Covered by SUPPORTED_COMMANDS check above.
        };

        Ok((agent_id, json_cmd.to_string()))
    }

    /// Handle commands that are executed locally (not via UDS) (#559).
    /// Returns `Some(result)` if the command was handled, `None` to fall through.
    fn try_local_command(&self, arguments: &str) -> Option<ToolResult> {
        let args: serde_json::Value = serde_json::from_str(arguments).ok()?;
        let command = args.get("command").and_then(|v| v.as_str())?;
        if command != "kill" {
            return None;
        }
        let agent_id = match args.get("agent_id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => {
                return Some(ToolResult {
                    content: "agent_cmd error: missing required field: agent_id".into(),
                    is_error: true,
                    image_blocks: vec![],
                });
            }
        };
        if let Err(e) = validate_agent_id_format(agent_id) {
            return Some(ToolResult {
                content: format!("agent_cmd error: {e}"),
                is_error: true,
                image_blocks: vec![],
            });
        }
        Some(self.kill_agent(agent_id))
    }

    /// Kill a specific subagent by ID: SIGTERM + remove from registry (#559).
    fn kill_agent(&self, agent_id: &str) -> ToolResult {
        let mut entries = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        let entry = match entries.remove(agent_id) {
            Some(e) => e,
            None => {
                return ToolResult {
                    content: format!(
                        "agent_cmd error: subagent '{}' not found in registry",
                        agent_id
                    ),
                    is_error: true,
                    image_blocks: vec![],
                };
            }
        };

        // Abort monitor task if running.
        if let Some(ref handle) = entry.monitor_handle {
            handle.abort();
        }

        // Send SIGTERM to the child process.
        if entry.pid != 0 {
            let _ = std::process::Command::new("kill")
                .arg("-TERM")
                .arg(entry.pid.to_string())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }

        ToolResult {
            content: format!("Subagent '{}' killed (pid={}).", agent_id, entry.pid),
            is_error: false,
            image_blocks: vec![],
        }
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
    // Do NOT shutdown or drop the write half (#557). In multi-client mode,
    // the server's reader loop exits on EOF → aborts the broadcast writer
    // task → response is never delivered. `writer` must stay alive (even
    // unused) to keep the write half open until the response is read.
    let _keep_alive = writer;

    let mut lines = BufReader::new(reader).lines();

    // Read lines until we find a "response" event (#555).
    // In multi-client mode, the broadcast delivers all events to all clients.
    // Skip non-response events (tokens, agent_start, etc.).
    let deadline = tokio::time::Instant::now() + RESPONSE_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(DomainError::Tool(
                "subagent response timed out (300s)".into(),
            ));
        }
        let line = tokio::time::timeout(remaining, lines.next_line())
            .await
            .map_err(|_| DomainError::Tool("subagent response timed out (300s)".into()))?
            .map_err(|e| DomainError::Tool(format!("read from subagent failed: {e}")))?;

        match line {
            Some(l) => {
                // Parse to check event type — avoids false positives from substring matching.
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&l) {
                    if json.get("type").and_then(|v| v.as_str()) == Some("response") {
                        return Ok(l);
                    }
                }
                // Not a response event — skip.
            }
            None => {
                return Err(DomainError::Tool(
                    "subagent closed connection without sending a response".into(),
                ));
            }
        }
    }
}

impl Tool for AgentCmdTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "agent_cmd".into(),
            description: "Send a command to a spawned subagent. \
                Supported commands: prompt, steer, follow_up, abort, kill, \
                get_state, get_messages, get_messages_tail, get_session_stats, \
                get_subagents, get_extensions, set_model, clear_history, \
                reload_extensions."
                .into(),
            parameters_schema: r#"{"type":"object","properties":{"agent_id":{"type":"string","description":"ID of the spawned subagent"},"command":{"type":"string","enum":["prompt","steer","follow_up","abort","kill","get_state","get_messages","get_messages_tail","get_session_stats","get_subagents","get_extensions","set_model","clear_history","reload_extensions"],"description":"Command to send (kill terminates the subagent process)"},"message":{"type":"string","description":"Message for prompt/steer/follow_up commands"},"count":{"type":"integer","description":"Number of messages for get_messages_tail (default: 1)"},"model":{"type":"string","description":"Model identifier for set_model (e.g. provider/modelId)"},"provider":{"type":"string","description":"Provider name for set_model (alternative to model)"},"model_id":{"type":"string","description":"Model ID for set_model (used with provider)"}},"required":["agent_id","command"]}"#.into(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let args = arguments.to_string();
        Box::pin(async move {
            // Check for locally-handled commands first (#559).
            if let Some(result) = self.try_local_command(&args) {
                return Ok(result);
            }

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
#[path = "agent_cmd_tests.rs"]
mod tests;
