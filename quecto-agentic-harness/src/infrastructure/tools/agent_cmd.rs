// Agent command tool: native UDS interaction with spawned subagents (#421).
//
// Connects to child agent UDS sockets directly from Rust — no ncat, no socat,
// no bash intermediary.  Uses the framed JSON protocol from
// `src/interface/cli/protocol.rs`.

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use std::future::Future;
use std::pin::Pin;

// Re-export shared types for external consumers.
pub use super::subagent_registry::{
    ExitSignalRx, SubagentEntry, SubagentRegistry, WorkflowSnapshot, new_registry,
    validate_agent_id_format,
};

/// Tool that sends UDS commands to spawned subagents.
///
/// Looks up the socket path from a shared [`SubagentRegistry`], connects,
/// sends the framed JSON command, reads the response, and returns it as a
/// structured [`ToolResult`].
#[derive(Debug, Clone)]
pub struct AgentCmdTool {
    /// Shared registry populated by [`super::spawn::SpawnTool`].
    registry: SubagentRegistry,
    /// Broadcast channel used to announce a `subagent_state_changed` survivor set
    /// when `kill` cascade-removes an agent's sub-tree, so connected clients (the
    /// TUI panel) drop the dead agents promptly (#831).
    broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
    /// Environment control use case for `get_containers` / `kill_container`
    /// (#1369 slice 2). This tool only decodes/delegates/encodes.
    environment_control:
        Option<std::sync::Arc<crate::environment_control_app::EnvironmentControlUseCase>>,
}

impl AgentCmdTool {
    /// Create a new `AgentCmdTool` backed by the given registry.
    pub fn new(registry: SubagentRegistry) -> Self {
        Self {
            registry,
            broadcast_tx: None,
            environment_control: None,
        }
    }

    /// Attach the session's environment control use case so `get_containers`
    /// and `kill_container` can delegate to it (#1369 slice 2).
    pub fn with_environment_control(
        mut self,
        environment_control: std::sync::Arc<
            crate::environment_control_app::EnvironmentControlUseCase,
        >,
    ) -> Self {
        self.environment_control = Some(environment_control);
        self
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

    /// Parse arguments and build the JSON command to send. Test-only wrapper
    /// over [`build_command`]; the dispatch path parses once and calls
    /// `build_command` directly.
    #[cfg(test)]
    fn parse_and_build(&self, arguments: &str) -> Result<(String, String, String), String> {
        let args: serde_json::Value =
            serde_json::from_str(arguments).map_err(|e| format!("invalid JSON: {e}"))?;
        super::agent_cmd_parse::build_command(&args)
    }

    /// Handle commands that are executed locally (not via UDS) (#559).
    /// Returns `Some(result)` if the command was handled synchronously,
    /// `None` to fall through to UDS dispatch.
    fn try_local_command(&self, args: &serde_json::Value) -> Option<ToolResult> {
        let command = args.get("command").and_then(|v| v.as_str())?;
        if command == "get_subagents_all" {
            return Some(self.list_all_subagents());
        }
        None
    }

    /// Handle the `kill` command (async: container teardown scripts run on a
    /// blocking worker and are awaited).
    async fn try_kill_command(&self, args: &serde_json::Value) -> Option<ToolResult> {
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
        Some(self.kill_agent(agent_id).await)
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
    async fn kill_agent(&self, agent_id: &str) -> ToolResult {
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

        let mut removed: Vec<_> = removed.into_iter().collect();
        // Parent-initiated kill is not a post-mortem: skip inspect, same as
        // kill_container's member teardown.
        super::subagent_cleanup::cleanup_removed_entries_once(
            &mut removed,
            super::subagent_cleanup::FinalizeMode::ParentKill,
        )
        .await;

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
            // Signal any lifecycle observers that the process exited.
            if let Some(ref tx) = entry.exit_signal_tx {
                let _ = tx.send(Some(super::subagent_registry::ExitSignal {
                    exit_code: None,
                    signal: Some(15), // SIGTERM
                    kind: Default::default(),
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

#[cfg(test)]
use super::agent_cmd_parse::SUPPORTED_COMMANDS;
use super::agent_cmd_report::{
    bounded_report_messages, is_substantive_assistant, needs_default_report_backfill,
};
use super::subagent_registry::PendingMessageReport;

impl AgentCmdTool {
    async fn expand_default_get_messages_response(
        &self,
        socket_path: &std::path::Path,
        routed_target_id: Option<&str>,
        first_response: &str,
        agent_id: &str,
    ) -> String {
        let delivered = {
            let entries = self.registry.lock().unwrap_or_else(|e| e.into_inner());
            super::subagent_registry::resolve_registry_key(&entries, agent_id)
                .ok()
                .and_then(|key| entries.get(&key).and_then(|e| e.delivered_message_ordinal))
                .unwrap_or(0)
        };
        let mut envelope = match serde_json::from_str::<serde_json::Value>(first_response) {
            Ok(v) => v,
            Err(_) => return first_response.to_string(),
        };
        let mut messages = envelope
            .pointer("/data/messages")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        const MAX_DEFAULT_REPORT_BACKFILL_PAGES: usize = 16;
        let mut backfill_complete = true;
        let mut backfill_pages = 0;
        while needs_default_report_backfill(&messages, delivered) {
            if backfill_pages >= MAX_DEFAULT_REPORT_BACKFILL_PAGES {
                backfill_complete = false;
                break;
            }
            backfill_pages += 1;
            let Some(before) = envelope.pointer("/data/before").and_then(|v| v.as_str()) else {
                backfill_complete = false;
                break;
            };
            let mut cmd = serde_json::json!({"type":"get_messages", "before": before});
            if let Some(target_id) = routed_target_id {
                cmd["agent_id"] = serde_json::json!(target_id);
            }
            let cmd = cmd.to_string();
            let Ok(line) = send_uds_command_with_timeout(
                socket_path,
                &cmd,
                super::subagent_registry::INSPECTOR_RESPONSE_TIMEOUT,
            )
            .await
            else {
                backfill_complete = false;
                break;
            };
            let Ok(older) = serde_json::from_str::<serde_json::Value>(&line) else {
                backfill_complete = false;
                break;
            };
            let older_messages = older
                .pointer("/data/messages")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if older_messages.is_empty() {
                backfill_complete = false;
                break;
            }
            messages.splice(0..0, older_messages);
            envelope = older;
        }
        if let Some(data) = envelope.get_mut("data") {
            data["messages"] = serde_json::Value::Array(messages);
            if !backfill_complete {
                data["reportIncomplete"] = serde_json::json!(true);
            }
        }
        envelope.to_string()
    }

    fn shape_default_get_messages_report(&self, agent_id: &str, response: &str) -> String {
        let Ok(mut envelope) = serde_json::from_str::<serde_json::Value>(response) else {
            return response.to_string();
        };
        if envelope.get("success").and_then(|v| v.as_bool()) == Some(false) {
            return response.to_string();
        }
        let Some(data) = envelope.get_mut("data") else {
            return response.to_string();
        };
        let report_incomplete =
            data.get("reportIncomplete").and_then(|v| v.as_bool()) == Some(true);
        let Some(messages) = data.get_mut("messages").and_then(|v| v.as_array_mut()) else {
            return response.to_string();
        };
        let mut entries = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        let Ok(key) = super::subagent_registry::resolve_registry_key(&entries, agent_id) else {
            return response.to_string();
        };
        let Some(entry) = entries.get_mut(&key) else {
            return response.to_string();
        };
        entry.pending_message_ordinal = None;
        let delivered = entry.delivered_message_ordinal.unwrap_or(0);
        let observed_max = messages
            .iter()
            .filter_map(|m| m.get("ordinal").and_then(|v| v.as_u64()))
            .max()
            .unwrap_or(0);
        if report_incomplete {
            if observed_max > delivered {
                let report = bounded_report_messages(
                    messages
                        .iter()
                        .filter(|m| {
                            m.get("ordinal")
                                .and_then(|v| v.as_u64())
                                .is_some_and(|ord| ord > delivered)
                        })
                        .cloned()
                        .collect(),
                    observed_max,
                );
                if !report.messages.is_empty() {
                    *data = serde_json::json!({"messages": report.messages, "truncated": true, "hasMoreMessages": report.has_more_messages, "messageContentTruncated": report.message_content_truncated, "reportIncomplete": true});
                    return envelope.to_string();
                }
            }
            *data = serde_json::json!({"unchanged": true, "reportIncomplete": true});
            return envelope.to_string();
        }
        if observed_max < delivered {
            // Durable append-time ordinals must not reset after reload or compaction.
            // A stale/partial child response with only lower ordinals is therefore
            // treated as no new default report rather than rewinding the cursor.
            *data = serde_json::json!({"unchanged": true});
            return envelope.to_string();
        }
        let mut max_ord = delivered;
        let mut latest_assistant: Option<usize> = None;
        let mut unread = Vec::new();
        for (idx, msg) in messages.iter_mut().enumerate() {
            let ord = msg
                .get("ordinal")
                .and_then(|v| v.as_u64())
                .unwrap_or_else(|| {
                    let next = max_ord.saturating_add(1);
                    msg["ordinal"] = serde_json::json!(next);
                    next
                });
            max_ord = max_ord.max(ord);
            if ord > delivered {
                unread.push(idx);
                if is_substantive_assistant(msg) {
                    latest_assistant = Some(idx);
                }
            }
        }
        if unread.is_empty() {
            *data = serde_json::json!({"unchanged": true});
            return envelope.to_string();
        }
        let candidates: Vec<_> = if delivered == 0 {
            latest_assistant
                .into_iter()
                .map(|i| messages[i].clone())
                .collect()
        } else {
            unread.into_iter().map(|i| messages[i].clone()).collect()
        };
        if candidates.is_empty() {
            *data = serde_json::json!({"unchanged": true});
            return envelope.to_string();
        }
        let report = bounded_report_messages(candidates, max_ord);
        if report.messages.is_empty() {
            if max_ord > delivered {
                *data = serde_json::json!({
                    "messages": [],
                    "truncated": true,
                    "hasMoreMessages": true,
                    "messageContentTruncated": false,
                    "reportIncomplete": true
                });
            } else {
                *data = serde_json::json!({"unchanged": true});
            }
            return envelope.to_string();
        }
        let new_cursor = report
            .messages
            .iter()
            .filter_map(|m| m.get("ordinal").and_then(|v| v.as_u64()))
            .max()
            .unwrap_or(delivered);
        let truncated = report.has_more_messages || report.message_content_truncated;
        *data = serde_json::json!({
            "messages": report.messages,
            "truncated": truncated,
            "hasMoreMessages": report.has_more_messages,
            "messageContentTruncated": report.message_content_truncated
        });
        let shaped = envelope.to_string();
        entry
            .pending_message_reports
            .push_back(PendingMessageReport {
                response: shaped.clone(),
                ordinal: new_cursor,
            });
        entry.pending_message_ordinal = Some(new_cursor);
        shaped
    }
}

use super::subagent_registry::send_subagent_uds_command as send_uds_command;
use super::subagent_registry::send_subagent_uds_command_with_timeout as send_uds_command_with_timeout;

impl Tool for AgentCmdTool {
    fn result_delivered(&self, arguments: &str, result: &ToolResult) {
        let Ok(args) = serde_json::from_str::<serde_json::Value>(arguments) else {
            return;
        };
        if result.is_error {
            return;
        }
        let command = args.get("command").and_then(|v| v.as_str());
        if !matches!(command, Some("get_messages") | Some("clear_history")) {
            return;
        }
        let Some(agent_id) = args.get("agent_id").and_then(|v| v.as_str()) else {
            return;
        };
        let mut entries = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        let Ok(key) = super::subagent_registry::resolve_registry_key(&entries, agent_id) else {
            return;
        };
        let Some(entry) = entries.get_mut(&key) else {
            return;
        };
        if command == Some("clear_history") {
            let succeeded = serde_json::from_str::<serde_json::Value>(&result.content)
                .ok()
                .and_then(|value| value.get("success").and_then(|v| v.as_bool()))
                == Some(true);
            if succeeded {
                entry.delivered_message_ordinal = None;
                entry.pending_message_ordinal = None;
                entry.pending_message_reports.clear();
            }
            return;
        }
        if !args.get("count").is_none_or(|v| v.is_null())
            || !args.get("before").is_none_or(|v| v.is_null())
        {
            return;
        }
        let delivered = serde_json::from_str::<serde_json::Value>(&result.content).ok();
        let delivered_success = delivered
            .as_ref()
            .and_then(|value| value.get("success").and_then(|v| v.as_bool()))
            == Some(true);
        let report_incomplete = delivered
            .as_ref()
            .and_then(|value| value.pointer("/data/reportIncomplete"))
            .and_then(|v| v.as_bool())
            == Some(true);
        if !delivered_success || report_incomplete {
            entry
                .pending_message_reports
                .retain(|pending| pending.response != result.content);
            entry.pending_message_ordinal = entry.pending_message_reports.back().map(|p| p.ordinal);
            return;
        }
        let pending = entry
            .pending_message_reports
            .iter()
            .position(|pending| pending.response == result.content)
            .and_then(|idx| entry.pending_message_reports.remove(idx));
        if let Some(pending) = pending {
            entry.delivered_message_ordinal = Some(
                entry
                    .delivered_message_ordinal
                    .unwrap_or(0)
                    .max(pending.ordinal),
            );
        }
        entry.pending_message_ordinal = entry.pending_message_reports.back().map(|p| p.ordinal);
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "agent_cmd".into(),
            description: "Send a command to a spawned subagent.                 Supported commands: prompt, steer, follow_up, abort, kill,                 get_state, get_messages, get_message, get_session_stats,                 get_subagents, get_subagents_all, get_containers, kill_container,                 get_tool_catalogue, set_model, set_effort, clear_history.                 COMPLETION SEQUENCE (required): (1) spawn returns when the socket is ready —                 do not wait in this turn. (2) End your turn or do other non-blocking parent                 work; do NOT poll get_subagents/get_subagents_all/get_state in a loop, do NOT                 sleep/bash-wait for the child. (3) On your NEXT turn a passive one-line                 completion note arrives automatically. (4) Then agent_cmd get_messages                 for the default unread report — the note is not the report.                 get_subagents_all is for inventory/cleanup after work, not completion waiting.                 get_state is the live/in-flight supervision API (occasional progress/debug,                 not a wait loop). Plain get_messages is the default unread report; explicit count/before                 request cursor-neutral history pages. Busy get_messages may be snapshot:true and lag."
                .into(),
            parameters_schema: r#"{"type":"object","properties":{"agent_id":{"type":"string","description":"ID of the spawned subagent; use '*' for command=get_subagents_all"},"command":{"type":"string","enum":["prompt","steer","follow_up","abort","kill","get_state","get_messages","get_message","get_session_stats","get_subagents","get_subagents_all","get_containers","kill_container","get_tool_catalogue","set_model","set_effort","clear_history"],"description":"Command to send. After spawn, wait for the passive completion note on a later turn — do not poll get_subagents* or sleep. Then get_messages for the default unread report. get_subagents_all is inventory/cleanup (agent_id '*'), not a wait loop. kill terminates the child process."},"message":{"type":"string","description":"Message for prompt/steer/follow_up commands"},"count":{"type":"integer","description":"Explicit history page size for get_messages; omit/null for the default unread report; does not move the report cursor"},"since":{"type":"integer","description":"Optional get_state generation cursor. If unchanged, the child returns only {\"unchanged\":true,\"generation\":N}."},"before":{"type":"string","description":"Paging cursor for get_messages (#1061): a message id from a prior response's before field; returns the adjacent older page"},"messageId":{"type":"string","description":"Stable message id for get_message/contentRecovery"},"offset":{"type":"integer","description":"Byte offset for get_message content recovery"},"limit":{"type":"integer","description":"Optional byte limit for get_message content recovery"},"toolCallId":{"type":"string","description":"Optional tool call id for get_message"},"model":{"type":"string","description":"Model identifier for set_model (e.g. provider/modelId)"},"provider":{"type":"string","description":"Provider name for set_model (alternative to model)"},"model_id":{"type":"string","description":"Model ID for set_model (used with provider)"},"effort":{"type":"string","description":"Effort level for set_effort: none, low, medium, high, xhigh, max"},"ref":{"type":"string","description":"Environment ref (e.g. C1) for kill_container (agent_id '*')"},"name":{"type":"string","description":"Environment name for kill_container (alternative to ref)"}},"required":["agent_id","command"]}"#
                .into(),
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
                // Session-level container commands decode/delegate/encode via
                // the environment control use case (#1369 slice 2).
                if super::agent_cmd_containers::is_container_command(value) {
                    return Ok(super::agent_cmd_containers::execute_container_command(
                        self.environment_control.as_ref(),
                        value,
                    )
                    .await);
                }
                // Check for sync locally-handled commands (#559).
                if let Some(result) = self.try_local_command(value) {
                    return Ok(result);
                }
                // kill is local but async: environment teardown scripts must
                // run off the runtime thread and be awaited (#1369 slice 2).
                if let Some(result) = self.try_kill_command(value).await {
                    return Ok(result);
                }
            }

            // Validate arguments and build the command.
            let (agent_id, json_cmd, command) = match &parsed {
                Ok(value) => match super::agent_cmd_parse::build_command(value) {
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

            let default_get_messages_report = parsed.as_ref().ok().is_some_and(|value| {
                value.get("command").and_then(|v| v.as_str()) == Some("get_messages")
                    && value.get("count").is_none_or(|v| v.is_null())
                    && value.get("before").is_none_or(|v| v.is_null())
            });

            let route = if let Some(routable) =
                super::subagent_routing::RoutableInspectionCommand::from_agent_cmd(&command)
            {
                debug_assert!(matches!(
                    routable,
                    super::subagent_routing::RoutableInspectionCommand::GetMessages
                        | super::subagent_routing::RoutableInspectionCommand::GetMessage
                        | super::subagent_routing::RoutableInspectionCommand::GetState
                ));
                super::subagent_routing::resolve_inspection_route(&self.registry, &agent_id)
            } else {
                self.lookup_socket(&agent_id).map(|socket_path| {
                    super::subagent_routing::InspectionRoute::Direct { socket_path }
                })
            };
            // Look up the socket.
            let route = match route {
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
            let mut json_cmd = json_cmd;
            let mut routed_target_id: Option<String> = None;
            let socket_path = match &route {
                super::subagent_routing::InspectionRoute::Direct { socket_path } => socket_path,
                super::subagent_routing::InspectionRoute::ViaAncestor {
                    ancestor_socket_path,
                    target_id,
                    ..
                } => {
                    routed_target_id = Some(target_id.clone());
                    if let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&json_cmd) {
                        value["agent_id"] = serde_json::json!(target_id);
                        json_cmd = value.to_string();
                    }
                    ancestor_socket_path
                }
            };
            let send = if Self::is_control_command(&command) {
                send_uds_command_with_timeout(
                    socket_path,
                    &json_cmd,
                    super::subagent_registry::INSPECTOR_RESPONSE_TIMEOUT,
                )
                .await
            } else {
                send_uds_command(socket_path, &json_cmd).await
            };

            // Send the command via UDS. Lifecycle state comes from the child's
            // monitor events; the transport ack alone cannot prove accepted work
            // and must not race with `agent_end` by marking the child Busy here.
            match send {
                Ok(response) => {
                    let response = if default_get_messages_report {
                        self.expand_default_get_messages_response(
                            socket_path,
                            routed_target_id.as_deref(),
                            &response,
                            &agent_id,
                        )
                        .await
                    } else {
                        response
                    };
                    let content = if default_get_messages_report {
                        self.shape_default_get_messages_report(&agent_id, &response)
                    } else {
                        response
                    };
                    Ok(ToolResult {
                        content,
                        is_error: false,
                        image_blocks: vec![],
                    })
                }
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
#[path = "agent_cmd_definition_tests.rs"]
mod definition_tests;
#[cfg(test)]
#[path = "agent_cmd_get_subagents_all_tests.rs"]
mod get_subagents_all_tests;
#[cfg(test)]
#[path = "agent_cmd_recovery_tests.rs"]
mod recovery_tests;
#[cfg(test)]
#[path = "agent_cmd_report_tests.rs"]
mod report_tests;
#[cfg(test)]
#[path = "agent_cmd_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "agent_cmd_876_tests.rs"]
mod tests_876;
