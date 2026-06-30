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
    /// Broadcast channel used to announce a `subagent_state_changed` survivor set
    /// when `kill` cascade-removes an agent's sub-tree, so connected clients (the
    /// TUI panel) drop the dead agents promptly (#831).
    broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
}

impl AgentCmdTool {
    /// Create a new `AgentCmdTool` backed by the given registry.
    pub fn new(registry: SubagentRegistry) -> Self {
        Self {
            registry,
            active_awaits: new_active_awaits(),
            broadcast_tx: None,
        }
    }

    /// Create with both a registry and a shared active_awaits tracker.
    pub fn with_active_awaits(registry: SubagentRegistry, active_awaits: ActiveAwaits) -> Self {
        Self {
            registry,
            active_awaits,
            broadcast_tx: None,
        }
    }

    /// Attach the broadcast channel so `kill` can announce the survivor set after
    /// a cascade-remove (#831). Best-effort: a send with no subscribers is fine.
    pub fn with_broadcast(
        mut self,
        broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
    ) -> Self {
        self.broadcast_tx = broadcast_tx;
        self
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
    fn parse_and_build(&self, arguments: &str) -> Result<(String, String, String), String> {
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

        if !SUPPORTED_COMMANDS.contains(&command.as_str()) && command != "get_messages_tail" {
            return Err(format!(
                "unsupported command '{}'; supported: {}",
                command,
                SUPPORTED_COMMANDS.join(", ")
            ));
        }

        // Build the JSON-lines command. Control commands (prompt/steer/
        // follow_up/abort) carry `"ack":"accept"` so a BUSY child's reader acks
        // ACCEPTANCE immediately instead of leaving the parent frozen until the
        // child's turn completes (#876); completion still arrives via the
        // auto-await note / `await`.
        let json_cmd = match command.as_str() {
            "prompt" => {
                let message = args
                    .get("message")
                    .and_then(|v| v.as_str())
                    .ok_or("prompt command requires a message field")?;
                serde_json::json!({"type": "prompt", "message": message, "ack": "accept"})
            }
            "steer" => {
                let message = args
                    .get("message")
                    .and_then(|v| v.as_str())
                    .ok_or("steer command requires a message field")?;
                serde_json::json!({"type": "steer", "message": message, "ack": "accept"})
            }
            "follow_up" => {
                let message = args
                    .get("message")
                    .and_then(|v| v.as_str())
                    .ok_or("follow_up command requires a message field")?;
                serde_json::json!({"type": "follow_up", "message": message, "ack": "accept"})
            }
            "get_state" => serde_json::json!({"type": "get_state"}),
            "get_messages" => {
                let mut cmd = serde_json::json!({"type": "get_messages"});
                if let Some(count) = args.get("count").and_then(|v| v.as_u64()) {
                    cmd["count"] = serde_json::json!(count);
                }
                cmd
            }
            "get_messages_tail" => {
                let count = args.get("count").and_then(|v| v.as_u64()).unwrap_or(1);
                serde_json::json!({"type": "get_messages", "count": count})
            }
            "abort" => serde_json::json!({"type": "abort", "ack": "accept"}),
            "get_session_stats" => serde_json::json!({"type": "get_session_stats"}),
            "set_model" => {
                // Reuse the shared model-arg validation (#881) so `set_model`
                // and `spawn`'s `model` cannot diverge.
                use crate::domain::subagent::{ModelArg, parse_model_arg};
                let parsed = parse_model_arg(
                    args.get("model").and_then(|v| v.as_str()),
                    args.get("provider").and_then(|v| v.as_str()),
                    args.get("model_id").and_then(|v| v.as_str()),
                )
                .map_err(|e| format!("set_model: {e}"))?;
                match parsed {
                    Some(ModelArg::Full(m)) => {
                        serde_json::json!({"type": "set_model", "model": m, "ack": "accept"})
                    }
                    Some(ModelArg::Pair { provider, model_id }) => {
                        serde_json::json!({"type": "set_model", "provider": provider, "modelId": model_id, "ack": "accept"})
                    }
                    None => {
                        return Err("set_model requires model, or provider + model_id".to_string());
                    }
                }
            }
            "clear_history" => serde_json::json!({"type": "clear_history", "ack": "accept"}),
            "get_subagents" => serde_json::json!({"type": "get_subagents"}),
            "get_extensions" => serde_json::json!({"type": "get_extensions"}),
            "reload_extensions" => {
                serde_json::json!({"type": "reload_extensions", "ack": "accept"})
            }
            "kill" => return Err("kill command is handled locally, not via UDS".to_string()),
            "await" => return Err("await command is handled locally, not via UDS".to_string()),
            _ => unreachable!(), // Covered by SUPPORTED_COMMANDS check above.
        };

        Ok((agent_id, json_cmd.to_string(), command))
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

    /// Queueable forwarded commands carry `"ack":"accept"` — the child acks
    /// ACCEPTANCE promptly (its reader, not the blocked dispatch loop), so the
    /// parent waits only the short interactive timeout, never the 300s
    /// turn-completion deadline (#876/#880).
    fn is_control_command(command: &str) -> bool {
        matches!(
            command,
            "prompt"
                | "steer"
                | "follow_up"
                | "abort"
                | "set_model"
                | "clear_history"
                | "reload_extensions"
        )
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

    /// Kill a specific subagent by ID: SIGTERM + cascade-remove its sub-tree from
    /// the registry, then broadcast the survivor set (#559, #831).
    fn kill_agent(&self, agent_id: &str) -> ToolResult {
        // Cascade-remove the agent AND every descendant in one shot, getting back
        // the removed entries (for process cleanup) and a survivor-only
        // `subagent_state_changed` event (#831).
        let super::subagent_cascade::CascadeOutcome { removed, event } =
            super::subagent_cascade::cascade_remove_and_state_changed(&self.registry, agent_id);

        if removed.is_empty() {
            return ToolResult {
                content: format!(
                    "agent_cmd error: subagent '{}' not found in registry",
                    agent_id
                ),
                is_error: true,
                image_blocks: vec![],
            };
        }

        // Broadcast the survivor set so the TUI panel drops the whole dead
        // sub-tree promptly (#831). Best-effort send: no subscribers is fine.
        if let Some(event) = event {
            if let Some(tx) = &self.broadcast_tx {
                if let Err(e) = tx.send(event) {
                    tracing::debug!(
                        agent = %agent_id,
                        error = %e,
                        "kill: no subscribers for cascade state_changed broadcast"
                    );
                }
            }
        }

        // Terminate EVERY removed agent's process + monitor, not just the named
        // one (#831 security review): otherwise killing a parent would drop its
        // descendants from the registry while leaving their OS processes running
        // as untracked orphans that `shutdown_all` can no longer reach.
        let mut killed_pid = 0;
        for (id, entry) in &removed {
            if id == agent_id {
                killed_pid = entry.pid;
            }
            // Signal any waiting `await` call so it returns "exited" instead of
            // spinning until timeout (#612).
            if let Some(ref tx) = entry.exit_signal_tx {
                let _ = tx.send(Some(super::subagent_registry::ExitSignal {
                    exit_code: None,
                    signal: Some(15), // SIGTERM
                }));
            }
            // Abort the monitor task and SIGTERM the child process. The reaper
            // task spawned by SpawnTool will wait() each child.
            super::subagent_cascade::terminate_removed_entry(entry);
        }

        ToolResult {
            content: format!("Subagent '{}' killed (pid={}).", agent_id, killed_pid),
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
use super::subagent_registry::send_subagent_uds_command_with_timeout as send_uds_command_with_timeout;

impl Tool for AgentCmdTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "agent_cmd".into(),
            description: "Send a command to a spawned subagent. \
                Supported commands: prompt, steer, follow_up, abort, kill, await, \
                get_state, get_messages, get_session_stats, \
                get_subagents, get_extensions, set_model, clear_history, \
                reload_extensions. \
                Spawned subagents are auto-noted PASSIVELY: a one-line completion \
                note arrives WITHOUT blocking and enters your context at your NEXT \
                turn, so await is OPTIONAL. Use await only when you must BLOCK \
                synchronously until the sub-agent reaches idle, exited, timeout, or \
                error before continuing within the SAME turn; awaiting a completion \
                suppresses its duplicate auto-note. Either way, read the child's full \
                output explicitly with get_messages (optionally with count for the \
                last N messages) — the note/await summary is one line, not the result. \
                get_state reports live status/model/message counts; get_session_stats \
                reports token/cost accounting. While a child is BUSY mid-turn, \
                get_messages/get_state are served from a snapshot of its last \
                completed turn (tagged snapshot:true / isStreaming:true), so the \
                data may lag the in-flight turn."
                .into(),
            parameters_schema: r#"{"type":"object","properties":{"agent_id":{"type":"string","description":"ID of the spawned subagent"},"command":{"type":"string","enum":["prompt","steer","follow_up","abort","kill","await","get_state","get_messages","get_session_stats","get_subagents","get_extensions","set_model","clear_history","reload_extensions"],"description":"Command to send. kill terminates the subagent process. await blocks until idle, exited, timeout, or error; then inspect output with get_messages (use count for the last N messages)."},"message":{"type":"string","description":"Message for prompt/steer/follow_up commands"},"count":{"type":"integer","description":"Number of messages for get_messages (omit for all; N for last N)"},"model":{"type":"string","description":"Model identifier for set_model (e.g. provider/modelId)"},"provider":{"type":"string","description":"Provider name for set_model (alternative to model)"},"model_id":{"type":"string","description":"Model ID for set_model (used with provider)"},"timeout":{"type":"integer","description":"Maximum wall-clock seconds to wait for await command (default: 300)"},"idle_timeout":{"type":"integer","description":"Seconds the agent must stay idle before await returns (default: 5). Set to 0 for immediate return on first idle."}},"required":["agent_id","command"]}"#.into(),
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
            let (agent_id, json_cmd, command) = match self.parse_and_build(&args) {
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

            // Queueable forwards return on the child's acceptance ack, so cap
            // them at the short interactive timeout instead of the 300s
            // turn-completion deadline — the parent must never freeze its turn
            // for the child's full processing (#876/#880).
            // `command` is threaded from parse_and_build — no second args parse.
            let send = if Self::is_control_command(&command) {
                send_uds_command_with_timeout(
                    &socket_path,
                    &json_cmd,
                    super::subagent_registry::INSPECTOR_RESPONSE_TIMEOUT,
                )
                .await
            } else {
                send_uds_command(&socket_path, &json_cmd).await
            };

            // Send the command via UDS.
            match send {
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
#[path = "agent_cmd_await_exclusion_tests.rs"]
mod await_exclusion_tests;
#[cfg(test)]
#[path = "agent_cmd_definition_tests.rs"]
mod definition_tests;
#[cfg(test)]
#[path = "agent_cmd_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "agent_cmd_876_tests.rs"]
mod tests_876;
