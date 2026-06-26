// Agent command tool: native UDS interaction with spawned subagents (#421).
//
// Connects to child agent UDS sockets directly from Rust — no ncat, no socat,
// no bash intermediary.  Uses the existing JSON-lines protocol from
// `src/interface/cli/protocol.rs`.
//
// Extended with `await` command (#612) that blocks until a sub-agent reaches a
// terminal state (idle, exited, timeout, or error).

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};

// Re-export shared types for external consumers.
pub use super::subagent_registry::{
    ActiveAwaits, AwaitResult, ExitSignalRx, SubagentEntry, SubagentRegistry, WorkflowSnapshot,
    new_active_awaits, new_registry, validate_agent_id_format,
};

#[path = "agent_cmd_await.rs"]
mod agent_cmd_await;

/// Supported commands for interacting with a subagent.
const SUPPORTED_COMMANDS: &[&str] = &[
    "prompt",
    "steer",
    "follow_up",
    "abort",
    "kill",
    "await",
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

/// Default timeout for `await` command (seconds).
const AWAIT_DEFAULT_TIMEOUT: u64 = 300;

/// Maximum allowed timeout for `await` command (1 hour). Prevents DoS from
/// unbounded blocking when a hallucinating LLM passes u64::MAX.
const AWAIT_MAX_TIMEOUT: u64 = 3600;

/// Default idle_timeout for `await` command (seconds).
const AWAIT_DEFAULT_IDLE_TIMEOUT: u64 = 5;

/// Polling interval for checking subagent status during `await` (milliseconds).
/// Exit signals are handled via `tokio::select!` for instant wakeup, so this
/// only affects idle-timeout and registry-status polling.
const AWAIT_POLL_INTERVAL_MS: u64 = 500;

/// Tool that sends UDS commands to spawned subagents.
///
/// Looks up the socket path from a shared [`SubagentRegistry`], connects,
/// sends the JSON-lines command, reads the response, and returns it as a
/// structured [`ToolResult`].
#[derive(Debug, Clone)]
pub struct AgentCmdTool {
    /// Shared registry populated by [`super::spawn::SpawnTool`].
    registry: SubagentRegistry,
    /// Tracks active `await` calls to prevent duplicates (#612).
    active_awaits: ActiveAwaits,
}

impl AgentCmdTool {
    /// Create a new `AgentCmdTool` backed by the given registry.
    pub fn new(registry: SubagentRegistry) -> Self {
        Self {
            registry,
            active_awaits: new_active_awaits(),
        }
    }

    /// Create with both a registry and a shared active_awaits tracker.
    pub fn with_active_awaits(registry: SubagentRegistry, active_awaits: ActiveAwaits) -> Self {
        Self {
            registry,
            active_awaits,
        }
    }

    /// Create a new empty registry (convenience for tests and wiring).
    pub fn new_registry() -> SubagentRegistry {
        new_registry()
    }

    /// Return a reference to the active awaits tracker (for testing / wiring).
    pub fn active_awaits(&self) -> &ActiveAwaits {
        &self.active_awaits
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
            "kill" => return Err("kill command is handled locally, not via UDS".to_string()),
            "await" => return Err("await command is handled locally, not via UDS".to_string()),
            _ => unreachable!(), // Covered by SUPPORTED_COMMANDS check above.
        };

        Ok((agent_id, json_cmd.to_string()))
    }

    /// Handle commands that are executed locally (not via UDS) (#559, #612).
    /// Returns `Some(result)` if the command was handled synchronously,
    /// `None` to fall through to UDS dispatch.
    /// For async local commands (await), returns `None` but sets a flag —
    /// the caller must check `is_await_command` separately.
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

    /// Check if the arguments specify an `await` command.
    fn is_await_command(arguments: &str) -> bool {
        serde_json::from_str::<serde_json::Value>(arguments)
            .ok()
            .and_then(|v| {
                v.get("command")
                    .and_then(|c| c.as_str())
                    .map(|s| s == "await")
            })
            .unwrap_or(false)
    }

    /// Kill a specific subagent by ID: SIGTERM + remove from registry (#559).
    fn kill_agent(&self, agent_id: &str) -> ToolResult {
        // Extract entry then drop the lock before sending SIGTERM (#559 review).
        let entry = {
            let mut entries = self.registry.lock().unwrap_or_else(|e| e.into_inner());
            entries.remove(agent_id)
        };
        let entry = match entry {
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

        // Signal any waiting `await` call before dropping the entry (#612).
        // This ensures await returns "exited" instead of spinning until timeout.
        if let Some(ref tx) = entry.exit_signal_tx {
            let _ = tx.send(Some(super::subagent_registry::ExitSignal {
                exit_code: None,
                signal: Some(15), // SIGTERM
            }));
        }

        // Abort monitor task if running.
        if let Some(ref handle) = entry.monitor_handle {
            handle.abort();
        }

        // Send SIGTERM to the child process (lock already released).
        // The reaper task spawned by SpawnTool will wait() the child.
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
        super::subagent_registry::lookup_subagent_socket(&self.registry, agent_id)
    }
}

/// RAII guard that removes the agent_id from active_awaits when dropped (#612).
struct AwaitGuard {
    active_awaits: ActiveAwaits,
    agent_id: String,
}

impl Drop for AwaitGuard {
    fn drop(&mut self) {
        let mut active = self.active_awaits.lock().unwrap_or_else(|e| e.into_inner());
        active.remove(&self.agent_id);
    }
}

use super::subagent_registry::send_subagent_uds_command as send_uds_command;

impl Tool for AgentCmdTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "agent_cmd".into(),
            description: "Send a command to a spawned subagent. \
                Supported commands: prompt, steer, follow_up, abort, kill, await, \
                get_state, get_messages, get_messages_tail, get_session_stats, \
                get_subagents, get_extensions, set_model, clear_history, \
                reload_extensions. \
                Spawned subagents are auto-noted PASSIVELY: a one-line completion \
                note arrives WITHOUT blocking and enters your context at your NEXT \
                turn, so await is OPTIONAL. Use await only when you must BLOCK \
                synchronously until the sub-agent reaches idle, exited, timeout, or \
                error before continuing within the SAME turn; awaiting a completion \
                suppresses its duplicate auto-note. Either way, read the child's full \
                output explicitly with get_messages_tail or get_messages — the \
                note/await summary is one line, not the result."
                .into(),
            parameters_schema: r#"{"type":"object","properties":{"agent_id":{"type":"string","description":"ID of the spawned subagent"},"command":{"type":"string","enum":["prompt","steer","follow_up","abort","kill","await","get_state","get_messages","get_messages_tail","get_session_stats","get_subagents","get_extensions","set_model","clear_history","reload_extensions"],"description":"Command to send. kill terminates the subagent process. await blocks until idle, exited, timeout, or error; then inspect output with get_messages_tail or get_messages."},"message":{"type":"string","description":"Message for prompt/steer/follow_up commands"},"count":{"type":"integer","description":"Number of messages for get_messages_tail (default: 1)"},"model":{"type":"string","description":"Model identifier for set_model (e.g. provider/modelId)"},"provider":{"type":"string","description":"Provider name for set_model (alternative to model)"},"model_id":{"type":"string","description":"Model ID for set_model (used with provider)"},"timeout":{"type":"integer","description":"Maximum wall-clock seconds to wait for await command (default: 300)"},"idle_timeout":{"type":"integer","description":"Seconds the agent must stay idle before await returns (default: 5). Set to 0 for immediate return on first idle."}},"required":["agent_id","command"]}"#.into(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let args = arguments.to_string();
        Box::pin(async move {
            // Check for async local commands first (#612).
            if Self::is_await_command(&args) {
                return self.execute_await(&args).await;
            }

            // Check for sync locally-handled commands (#559).
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
