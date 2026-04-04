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

/// Timeout for reading a response from a subagent UDS socket.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(300);

/// Default timeout for `await` command (seconds).
const AWAIT_DEFAULT_TIMEOUT: u64 = 300;

/// Default idle_timeout for `await` command (seconds).
const AWAIT_DEFAULT_IDLE_TIMEOUT: u64 = 5;

/// Polling interval for checking subagent status during `await` (milliseconds).
const AWAIT_POLL_INTERVAL_MS: u64 = 200;

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
            .and_then(|v| v.get("command").and_then(|c| c.as_str()).map(|s| s == "await"))
            .unwrap_or(false)
    }

    /// Execute the `await` command: block until the sub-agent reaches a terminal
    /// condition, then return a structured [`AwaitResult`] as JSON (#612).
    async fn execute_await(&self, arguments: &str) -> Result<ToolResult, DomainError> {
        let args: serde_json::Value = serde_json::from_str(arguments)
            .map_err(|e| DomainError::Tool(format!("invalid JSON: {e}")))?;

        let agent_id = args
            .get("agent_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| DomainError::Tool("missing required field: agent_id".into()))?
            .to_string();

        if let Err(e) = validate_agent_id_format(&agent_id) {
            return Ok(ToolResult {
                content: format!("agent_cmd error: {e}"),
                is_error: true,
                image_blocks: vec![],
            });
        }

        let timeout_secs = args
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(AWAIT_DEFAULT_TIMEOUT);

        let idle_timeout_secs = args
            .get("idle_timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(AWAIT_DEFAULT_IDLE_TIMEOUT);

        let start = std::time::Instant::now();

        // Check if agent exists in registry.
        let (socket_path, exit_signal_rx) = {
            let entries = self.registry.lock().unwrap_or_else(|e| e.into_inner());
            match entries.get(&agent_id) {
                Some(entry) => {
                    let rx = entry.exit_signal_tx.as_ref().map(|tx| tx.subscribe());
                    (entry.socket_path.clone(), rx)
                }
                None => {
                    let result = AwaitResult {
                        status: "error".into(),
                        reason: Some("agent_not_found".into()),
                        agent_id: agent_id.clone(),
                        elapsed_ms: 0,
                        workflow: None,
                    };
                    return Ok(ToolResult {
                        content: serde_json::to_string(&result).unwrap(),
                        is_error: false,
                        image_blocks: vec![],
                    });
                }
            }
        };

        // Check for duplicate awaiters.
        {
            let mut active = self.active_awaits.lock().unwrap_or_else(|e| e.into_inner());
            if active.contains(&agent_id) {
                let result = AwaitResult {
                    status: "error".into(),
                    reason: Some("another_await_active".into()),
                    agent_id: agent_id.clone(),
                    elapsed_ms: 0,
                    workflow: None,
                };
                return Ok(ToolResult {
                    content: serde_json::to_string(&result).unwrap(),
                    is_error: false,
                    image_blocks: vec![],
                });
            }
            active.insert(agent_id.clone());
        }

        // Ensure we remove from active_awaits when done (RAII guard).
        let _guard = AwaitGuard {
            active_awaits: self.active_awaits.clone(),
            agent_id: agent_id.clone(),
        };

        // Check if socket is connectable (detect stale sockets early).
        // Use a synchronous non-blocking connect to avoid issues with tokio
        // single-threaded runtimes where async connect may not yield properly.
        let connectable = if socket_path.exists() {
            std::os::unix::net::UnixStream::connect(&socket_path).is_ok()
        } else {
            false
        };
        if !connectable {
            // Check if the entry is still in the registry (might have been
            // removed by the reaper between our lookup and here).
            let still_registered = {
                let entries = self.registry.lock().unwrap_or_else(|e| e.into_inner());
                entries.contains_key(&agent_id)
            };
            if still_registered {
                let result = AwaitResult {
                    status: "error".into(),
                    reason: Some("connection_failed".into()),
                    agent_id: agent_id.clone(),
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    workflow: None,
                };
                return Ok(ToolResult {
                    content: serde_json::to_string(&result).unwrap(),
                    is_error: false,
                    image_blocks: vec![],
                });
            } else {
                let result = AwaitResult {
                    status: "error".into(),
                    reason: Some("agent_not_found".into()),
                    agent_id: agent_id.clone(),
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    workflow: None,
                };
                return Ok(ToolResult {
                    content: serde_json::to_string(&result).unwrap(),
                    is_error: false,
                    image_blocks: vec![],
                });
            }
        }

        // Main await loop: poll status + listen for exit signals.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
        let mut idle_since: Option<tokio::time::Instant> = None;
        let mut poll_interval =
            tokio::time::interval(Duration::from_millis(AWAIT_POLL_INTERVAL_MS));
        poll_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        // Clone the exit signal receiver for use in the select loop.
        let mut exit_rx = exit_signal_rx;

        loop {
            // Check if we've exceeded the overall timeout.
            if tokio::time::Instant::now() >= deadline {
                let workflow = self.fetch_workflow_snapshot(&agent_id).await;
                let result = AwaitResult {
                    status: "timeout".into(),
                    reason: None,
                    agent_id: agent_id.clone(),
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    workflow,
                };
                return Ok(ToolResult {
                    content: serde_json::to_string(&result).unwrap(),
                    is_error: false,
                    image_blocks: vec![],
                });
            }

            // Check for process exit signal (non-blocking).
            if let Some(ref mut rx) = exit_rx {
                if let Ok(has_changed) = rx.has_changed() {
                    if has_changed {
                        let signal = rx.borrow_and_update().clone();
                        if let Some(exit_signal) = signal {
                            let (status, reason) = if let Some(code) = exit_signal.exit_code {
                                ("exited".to_string(), Some(format!("exit_code_{code}")))
                            } else if let Some(sig) = exit_signal.signal {
                                ("exited".to_string(), Some(format!("signal_{sig}")))
                            } else {
                                ("exited".to_string(), Some("exit_code_0".to_string()))
                            };
                            let result = AwaitResult {
                                status,
                                reason,
                                agent_id: agent_id.clone(),
                                elapsed_ms: start.elapsed().as_millis() as u64,
                                workflow: None,
                            };
                            return Ok(ToolResult {
                                content: serde_json::to_string(&result).unwrap(),
                                is_error: false,
                                image_blocks: vec![],
                            });
                        }
                    }
                }
            }

            // Poll the registry for current status.
            let current_status = {
                let entries = self.registry.lock().unwrap_or_else(|e| e.into_inner());
                entries.get(&agent_id).map(|e| e.status.clone())
            };

            match current_status {
                None | Some(super::subagent_registry::SubagentStatus::Exited) => {
                    // Agent removed from registry (exited and reaped) or marked
                    // as Exited. Try to read the exit signal for the actual exit
                    // code/signal; fall back to exit_code_0 if unavailable.
                    // Read directly from the exit_signal_tx in the registry
                    // entry (if still present) for a reliable read.
                    let reason = {
                        let entries = self.registry.lock().unwrap_or_else(|e| e.into_inner());
                        entries
                            .get(&agent_id)
                            .and_then(|e| e.exit_signal_tx.as_ref())
                            .and_then(|tx| {
                                // subscribe() gives us the current value.
                                let rx = tx.subscribe();
                                let signal = rx.borrow().clone();
                                signal
                            })
                            .map(|es| {
                                if let Some(code) = es.exit_code {
                                    format!("exit_code_{code}")
                                } else if let Some(sig) = es.signal {
                                    format!("signal_{sig}")
                                } else {
                                    "exit_code_0".into()
                                }
                            })
                            .or(Some("exit_code_0".into()))
                    };
                    let result = AwaitResult {
                        status: "exited".into(),
                        reason,
                        agent_id: agent_id.clone(),
                        elapsed_ms: start.elapsed().as_millis() as u64,
                        workflow: None,
                    };
                    return Ok(ToolResult {
                        content: serde_json::to_string(&result).unwrap(),
                        is_error: false,
                        image_blocks: vec![],
                    });
                }
                Some(super::subagent_registry::SubagentStatus::Idle) => {
                    // Agent is idle — start or continue the idle_timeout countdown.
                    let now = tokio::time::Instant::now();
                    match idle_since {
                        None => {
                            idle_since = Some(now);
                            if idle_timeout_secs == 0 {
                                // Immediate return on idle.
                                let workflow =
                                    self.fetch_workflow_snapshot(&agent_id).await;
                                let result = AwaitResult {
                                    status: "idle".into(),
                                    reason: Some("completed".into()),
                                    agent_id: agent_id.clone(),
                                    elapsed_ms: start.elapsed().as_millis() as u64,
                                    workflow,
                                };
                                return Ok(ToolResult {
                                    content: serde_json::to_string(&result).unwrap(),
                                    is_error: false,
                                    image_blocks: vec![],
                                });
                            }
                        }
                        Some(since) => {
                            let idle_duration = now.duration_since(since);
                            if idle_duration >= Duration::from_secs(idle_timeout_secs) {
                                // Stable idle — return.
                                let workflow =
                                    self.fetch_workflow_snapshot(&agent_id).await;
                                let result = AwaitResult {
                                    status: "idle".into(),
                                    reason: Some("completed".into()),
                                    agent_id: agent_id.clone(),
                                    elapsed_ms: start.elapsed().as_millis() as u64,
                                    workflow,
                                };
                                return Ok(ToolResult {
                                    content: serde_json::to_string(&result).unwrap(),
                                    is_error: false,
                                    image_blocks: vec![],
                                });
                            }
                        }
                    }
                }
                Some(_) => {
                    // Agent is running/starting/error — reset idle countdown.
                    idle_since = None;
                }
            }

            // Wait for next poll tick.
            poll_interval.tick().await;
        }
    }

    /// Fetch workflow state from a subagent via UDS `get_state` command.
    /// Returns `None` if the fetch fails or workflow is not enabled.
    /// Uses a short timeout (2s) to avoid blocking if the agent is unresponsive.
    async fn fetch_workflow_snapshot(&self, agent_id: &str) -> Option<WorkflowSnapshot> {
        let socket_path = self.lookup_socket(agent_id).ok()?;
        let cmd = serde_json::json!({"type": "get_state"}).to_string();
        let response = tokio::time::timeout(
            Duration::from_secs(2),
            send_uds_command(&socket_path, &cmd),
        )
        .await
        .ok()?
        .ok()?;
        let parsed: serde_json::Value = serde_json::from_str(&response).ok()?;
        let data = parsed.get("data")?;

        // Look for workflow state in the response.
        let workflow = data.get("workflow").or_else(|| data.get("workflowState"))?;
        let mode = workflow
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let steps_completed = workflow
            .get("steps_completed")
            .or_else(|| workflow.get("stepsCompleted"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let steps_total = workflow
            .get("steps_total")
            .or_else(|| workflow.get("stepsTotal"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        Some(WorkflowSnapshot {
            mode,
            steps_completed,
            steps_total,
        })
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
        let entries = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        entries
            .get(agent_id)
            .map(|e| e.socket_path.clone())
            .ok_or_else(|| format!("subagent '{}' not found in registry", agent_id))
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
                Supported commands: prompt, steer, follow_up, abort, kill, await, \
                get_state, get_messages, get_messages_tail, get_session_stats, \
                get_subagents, get_extensions, set_model, clear_history, \
                reload_extensions. \
                The await command blocks until the sub-agent reaches a terminal \
                state (idle, exited, timeout, or error)."
                .into(),
            parameters_schema: r#"{"type":"object","properties":{"agent_id":{"type":"string","description":"ID of the spawned subagent"},"command":{"type":"string","enum":["prompt","steer","follow_up","abort","kill","await","get_state","get_messages","get_messages_tail","get_session_stats","get_subagents","get_extensions","set_model","clear_history","reload_extensions"],"description":"Command to send (kill terminates the subagent process, await blocks until terminal state)"},"message":{"type":"string","description":"Message for prompt/steer/follow_up commands"},"count":{"type":"integer","description":"Number of messages for get_messages_tail (default: 1)"},"model":{"type":"string","description":"Model identifier for set_model (e.g. provider/modelId)"},"provider":{"type":"string","description":"Provider name for set_model (alternative to model)"},"model_id":{"type":"string","description":"Model ID for set_model (used with provider)"},"timeout":{"type":"integer","description":"Maximum wall-clock seconds to wait for await command (default: 300)"},"idle_timeout":{"type":"integer","description":"Seconds the agent must stay idle before await returns (default: 5). Set to 0 for immediate return on first idle."}},"required":["agent_id","command"]}"#.into(),
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
