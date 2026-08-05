// Agent command tool: native UDS interaction with spawned subagents (#421).
//
// Connects to child agent UDS sockets directly from Rust — no ncat, no socat,
// no bash intermediary.  Uses the framed JSON protocol from
// `src/interface/cli/protocol.rs`.
//
// Extended with `await` command (#612) that blocks until a sub-agent reaches a
// terminal state (idle, exited, timeout, or error).
//
// Short-term: `await` stays implemented and dispatchable, but is **hidden from
// the model-facing tool schema/description** so agents default to passive
// completion notes + get_messages. Flip [`AWAIT_VISIBLE_IN_SCHEMA`] to `true`
// (and restore the await wording in `definition` / `spawn`) to re-advertise it.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use crate::infrastructure::tools::container_script_cleanup::cleanup_container_environments_after_removal;

// Re-export shared types for external consumers.
pub use super::subagent_registry::{
    ActiveAwaits, AwaitResult, ExitSignalRx, SubagentEntry, SubagentRegistry, WorkflowSnapshot,
    new_active_awaits, new_registry, validate_agent_id_format,
};

#[path = "agent_cmd_await.rs"]
mod agent_cmd_await;

/// When `false`, `await` is omitted from the `agent_cmd` tool schema and
/// description (model cannot discover it). Implementation and dispatch remain.
/// Set to `true` to re-advertise blocking await to the LLM.
const AWAIT_VISIBLE_IN_SCHEMA: bool = false;

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
    "get_subagents_all",
    "get_tool_catalogue",
    "set_model",
    "set_effort",
    "clear_history",
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
/// sends the framed JSON command, reads the response, and returns it as a
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

    /// Parse arguments and build the JSON command to send. Test-only wrapper
    /// over [`build_command`]; the dispatch path parses once and calls
    /// `build_command` directly.
    #[cfg(test)]
    fn parse_and_build(&self, arguments: &str) -> Result<(String, String, String), String> {
        let args: serde_json::Value =
            serde_json::from_str(arguments).map_err(|e| format!("invalid JSON: {e}"))?;
        self.build_command(&args)
    }

    /// Validate the already-parsed arguments and build the JSON command to send.
    /// Used by the dispatch path, which parses the arguments once per call.
    fn build_command(&self, args: &serde_json::Value) -> Result<(String, String, String), String> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or("missing required field: command")?
            .to_string();

        let agent_id = args
            .get("agent_id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or("missing required field: agent_id")?;

        // Validate agent_id format (same rules as spawn). The synthetic `*` target
        // is accepted only for the parent-local get_subagents_all command.
        if command == "get_subagents_all" {
            if agent_id != "*" {
                return Err("get_subagents_all requires agent_id '*'".to_string());
            }
        } else {
            validate_agent_id_format(&agent_id)?;
        }

        if !SUPPORTED_COMMANDS.contains(&command.as_str()) && command != "get_messages_tail" {
            return Err(format!(
                "unsupported command '{}'; supported: {}",
                command,
                SUPPORTED_COMMANDS.join(", ")
            ));
        }

        // Build the framed JSON command. Control commands (prompt/steer/
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
                serde_json::json!({"type": "prompt", "message": message, "streamingBehavior": "steer", "ack": "accept"})
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
                // Paged history (#1061): follow a response's `before` cursor to
                // the adjacent older page — an uncounted request returns only
                // the newest bounded page, never the full history.
                if let Some(before) = args.get("before").and_then(|v| v.as_str()) {
                    cmd["before"] = serde_json::json!(before);
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
            "set_effort" => {
                let effort = args
                    .get("effort")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or("set_effort requires effort")?;
                if crate::domain::provider::EffortLevel::parse(effort).is_none() {
                    return Err(format!(
                        "invalid effort '{effort}'; valid values: {}",
                        crate::domain::provider::EffortLevel::VALID_VALUES
                    ));
                }
                serde_json::json!({"type": "set_effort", "effort": effort, "ack": "accept"})
            }
            "clear_history" => serde_json::json!({"type": "clear_history", "ack": "accept"}),
            "get_subagents" => serde_json::json!({"type": "get_subagents"}),
            "get_subagents_all" => {
                return Err("get_subagents_all is handled locally, not via UDS".to_string());
            }
            "get_tool_catalogue" | "list_tools" => {
                serde_json::json!({"type": "get_tool_catalogue"})
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
    fn try_local_command(&self, args: &serde_json::Value) -> Option<ToolResult> {
        let command = args.get("command").and_then(|v| v.as_str())?;
        if command == "get_subagents_all" {
            return Some(self.list_all_subagents());
        }
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
            "prompt" | "steer" | "follow_up" | "abort" | "set_model" | "clear_history"
        )
    }

    /// Check if the arguments specify an `await` command. Test-only wrapper over
    /// [`is_await_value`]; the dispatch path parses once and calls it directly.
    #[cfg(test)]
    fn is_await_command(arguments: &str) -> bool {
        serde_json::from_str::<serde_json::Value>(arguments)
            .ok()
            .as_ref()
            .is_some_and(Self::is_await_value)
    }

    /// Check if the already-parsed arguments specify an `await` command.
    fn is_await_value(args: &serde_json::Value) -> bool {
        args.get("command").and_then(|c| c.as_str()) == Some("await")
    }

    /// List every subagent currently tracked by this parent agent's registry.
    fn list_all_subagents(&self) -> ToolResult {
        let subagents =
            crate::interface::cli::protocol::build_subagent_info_list(&Some(self.registry.clone()));
        ToolResult {
            content: serde_json::json!({"subagents": subagents}).to_string(),
            is_error: false,
            image_blocks: vec![],
        }
    }

    /// Kill a specific subagent by ID: SIGTERM + cascade-remove its sub-tree from
    /// the registry, then broadcast the survivor set (#559, #831).
    fn kill_agent(&self, agent_id: &str) -> ToolResult {
        let registry_key = {
            let entries = self.registry.lock().unwrap_or_else(|e| e.into_inner());
            entries
                .iter()
                .find(|(key, entry)| {
                    key.as_str() == agent_id
                        || (entry.display_name == agent_id
                            && entry.status != super::subagent_registry::SubagentStatus::Exited)
                })
                .map(|(key, _)| key.clone())
        };
        let Some(registry_key) = registry_key else {
            return ToolResult {
                content: format!(
                    "agent_cmd error: subagent '{}' not found in registry",
                    agent_id
                ),
                is_error: true,
                image_blocks: vec![],
            };
        };

        // Cascade-remove the agent AND every descendant in one shot, getting back
        // the removed entries (for process cleanup) and a survivor-only
        // `subagent_state_changed` event (#831).
        let super::subagent_cascade::CascadeOutcome { removed, event } =
            super::subagent_cascade::cascade_remove_and_state_changed(
                &self.registry,
                &registry_key,
            );

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
        if let Err(err) =
            cleanup_container_environments_after_removal(&removed, &self.registry, None)
        {
            tracing::warn!(error = %err, "container cleanup failed after agent kill");
        }

        let mut killed_pid = 0;
        for (id, entry) in &removed {
            let mut lifecycle = entry.lifecycle;
            let killed_status = super::subagent_lifecycle::apply_lifecycle_event(
                &mut lifecycle,
                super::subagent_lifecycle::SubagentLifecycleEvent::KillRequested,
            );
            debug_assert_eq!(
                killed_status,
                super::subagent_registry::SubagentStatus::Exited,
                "kill must project to the existing exited status"
            );
            if id == &registry_key {
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

    /// Look up the socket path for an agent ID (legacy tests/TUI helpers).
    #[cfg(test)]
    fn lookup_socket(&self, agent_id: &str) -> Result<std::path::PathBuf, String> {
        super::subagent_registry::lookup_subagent_socket(&self.registry, agent_id)
    }

    /// Look up the typed parent endpoint for an agent ID.
    fn lookup_endpoint(
        &self,
        agent_id: &str,
    ) -> Result<crate::domain::agent_launch_backend::ParentEndpoint, String> {
        let socket_path =
            super::subagent_registry::lookup_subagent_socket(&self.registry, agent_id)?;
        let entries = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        let entry = entries
            .values()
            .find(|entry| entry.socket_path == socket_path)
            .ok_or_else(|| format!("agent '{agent_id}' endpoint disappeared"))?;
        super::parent_endpoint_guard::endpoint_or_proxy_error(entry, socket_path, agent_id)
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

#[cfg(test)]
use super::subagent_registry::send_subagent_uds_command as send_uds_command;

impl Tool for AgentCmdTool {
    fn definition(&self) -> ToolDefinition {
        // Schema/description only — see AWAIT_VISIBLE_IN_SCHEMA. Dispatch still
        // accepts await when the model invents the command name.
        let (description, parameters_schema) = if AWAIT_VISIBLE_IN_SCHEMA {
            (
                "Send a command to a spawned subagent. \
                Supported commands: prompt, steer, follow_up, abort, kill, await, \
                get_state, get_messages, get_session_stats, \
                get_subagents, get_subagents_all, get_tool_catalogue, set_model, clear_history. \
                Spawned subagents are auto-noted PASSIVELY: a one-line completion \
                note arrives WITHOUT blocking and enters your context at your NEXT \
                turn, so await is OPTIONAL. Use await only when you must BLOCK \
                synchronously until the sub-agent reaches idle, exited, timeout, or \
                error before continuing within the SAME turn; awaiting a completion \
                suppresses its duplicate auto-note. Either way, read the child's \
                output explicitly with get_messages — it returns the NEWEST bounded \
                history page (count for the last N); when the response reports \
                hasMoreBefore:true, pass its before cursor to page older history — \
                the note/await summary is one line, not the result. \
                get_state is the live/in-flight supervision API: it reports execution \
                phase, current/recent tool activity, progress, model, effort, and message \
                count. get_messages is the stable committed transcript API, intended for \
                full or end-of-turn output inspection. Busy responses are tagged \
                snapshot:true; transcript data may lag the active turn.",
                r#"{"type":"object","properties":{"agent_id":{"type":"string","description":"ID of the spawned subagent; use '*' for command=get_subagents_all"},"command":{"type":"string","enum":["prompt","steer","follow_up","abort","kill","await","get_state","get_messages","get_session_stats","get_subagents","get_subagents_all","get_tool_catalogue","set_model","set_effort","clear_history"],"description":"Command to send. get_subagents_all lists this parent agent's tracked subagents without targeting a child. kill terminates the subagent process. await blocks until idle, exited, timeout, or error; then inspect output with get_messages (use count for the last N messages)."},"message":{"type":"string","description":"Message for prompt/steer/follow_up commands"},"count":{"type":"integer","description":"Number of messages for get_messages (omit for the newest history page; N for last N)"},"before":{"type":"string","description":"Paging cursor for get_messages (#1061): a message id from a prior response's before field; returns the adjacent older page"},"model":{"type":"string","description":"Model identifier for set_model (e.g. provider/modelId)"},"provider":{"type":"string","description":"Provider name for set_model (alternative to model)"},"model_id":{"type":"string","description":"Model ID for set_model (used with provider)"},"effort":{"type":"string","description":"Effort level for set_effort: none, low, medium, high, xhigh, max"},"timeout":{"type":"integer","description":"Maximum wall-clock seconds to wait for await command (default: 300)"},"idle_timeout":{"type":"integer","description":"Seconds agent must stay idle before await returns (default: 5). Set to 0 for immediate return on first idle."}},"required":["agent_id","command"]}"#,
            )
        } else {
            (
                "Send a command to a spawned subagent. \
                Supported commands: prompt, steer, follow_up, abort, kill, \
                get_state, get_messages, get_session_stats, \
                get_subagents, get_subagents_all, get_tool_catalogue, set_model, clear_history. \
                COMPLETION SEQUENCE (required): (1) spawn returns when the socket is ready — \
                do not wait in this turn. (2) End your turn or do other non-blocking parent \
                work; do NOT poll get_subagents/get_subagents_all/get_state in a loop, do NOT \
                sleep/bash-wait for the child. (3) On your NEXT turn a passive one-line \
                completion note arrives automatically. (4) Then agent_cmd get_messages \
                (count 1-5) for the child's report — the note is not the report. \
                get_subagents_all is for inventory/cleanup after work, not completion waiting. \
                get_state is the live/in-flight supervision API (occasional progress/debug, \
                not a wait loop). get_messages is the stable committed transcript API \
                (newest bounded history page; count for last N; hasMoreBefore/before pages \
                older). Busy get_messages may be snapshot:true and lag.",
                r#"{"type":"object","properties":{"agent_id":{"type":"string","description":"ID of the spawned subagent; use '*' for command=get_subagents_all"},"command":{"type":"string","enum":["prompt","steer","follow_up","abort","kill","get_state","get_messages","get_session_stats","get_subagents","get_subagents_all","get_tool_catalogue","set_model","set_effort","clear_history"],"description":"Command to send. After spawn, wait for the passive completion note on a later turn — do not poll get_subagents* or sleep. Then get_messages (count 1-5) for the report. get_subagents_all is inventory/cleanup (agent_id '*'), not a wait loop. kill terminates the child process."},"message":{"type":"string","description":"Message for prompt/steer/follow_up commands"},"count":{"type":"integer","description":"Number of messages for get_messages (omit for the newest history page; N for last N)"},"before":{"type":"string","description":"Paging cursor for get_messages (#1061): a message id from a prior response's before field; returns the adjacent older page"},"model":{"type":"string","description":"Model identifier for set_model (e.g. provider/modelId)"},"provider":{"type":"string","description":"Provider name for set_model (alternative to model)"},"model_id":{"type":"string","description":"Model ID for set_model (used with provider)"},"effort":{"type":"string","description":"Effort level for set_effort: none, low, medium, high, xhigh, max"}},"required":["agent_id","command"]}"#,
            )
        };
        ToolDefinition {
            name: "agent_cmd".into(),
            description: description.into(),
            parameters_schema: parameters_schema.into(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let args = arguments.to_string();
        Box::pin(async move {
            // Parse the argument JSON exactly once and thread the parsed value
            // through every dispatch predicate (#996 item 4).
            let parsed = serde_json::from_str::<serde_json::Value>(&args);

            if let Ok(ref value) = parsed {
                // Check for async local commands first (#612).
                if Self::is_await_value(value) {
                    return self.execute_await(&args).await;
                }
                // Check for sync locally-handled commands (#559).
                if let Some(result) = self.try_local_command(value) {
                    return Ok(result);
                }
            }

            // Validate arguments and build the command.
            let (agent_id, json_cmd, command) = match &parsed {
                Ok(value) => match self.build_command(value) {
                    Ok(built) => built,
                    Err(e) => {
                        return Ok(ToolResult {
                            content: format!("agent_cmd error: {e}"),
                            is_error: true,
                            image_blocks: vec![],
                        });
                    }
                },
                Err(e) => {
                    return Ok(ToolResult {
                        content: format!("agent_cmd error: invalid JSON: {e}"),
                        is_error: true,
                        image_blocks: vec![],
                    });
                }
            };

            // Look up the typed parent endpoint. Proxy-backed containers must
            // route through the proxy endpoint rather than the requested child
            // UDS path that may only exist inside the container.
            let endpoint = match self.lookup_endpoint(&agent_id) {
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
                super::parent_endpoint::send_command_with_timeout(
                    &endpoint,
                    &json_cmd,
                    super::subagent_registry::INSPECTOR_RESPONSE_TIMEOUT,
                )
                .await
            } else {
                super::parent_endpoint::send_command_with_timeout(
                    &endpoint,
                    &json_cmd,
                    super::subagent_registry::SUBAGENT_RESPONSE_TIMEOUT,
                )
                .await
            };

            // Send the command via UDS. Lifecycle state comes from the child's
            // monitor events; the transport ack alone cannot prove accepted work
            // and must not race with `agent_end` by marking the child Busy here.
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
#[path = "agent_cmd_get_subagents_all_tests.rs"]
mod get_subagents_all_tests;
#[cfg(test)]
#[path = "agent_cmd_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "agent_cmd_876_tests.rs"]
mod tests_876;
