// Agent command tool: native UDS interaction with spawned subagents (#421).
//
// Connects to child agent UDS sockets directly from Rust — no ncat, no socat,
// no bash intermediary.  Uses the framed JSON protocol from
// `src/interface/cli/protocol.rs`.

use crate::application::tools::ports::Tool;
use crate::domain::error::DomainError;
use crate::domain::tool_policy::value_objects::tool::{ToolDefinition, ToolResult};
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
#[derive(Clone)]
pub struct AgentCmdTool {
    /// Shared registry populated by [`super::spawn::SpawnTool`].
    registry: SubagentRegistry,
    /// The owner of the `kill` command (#1936): the composed selected
    /// termination tool, installed by the interface after this tool is
    /// built. Empty, `kill` is refused: this tool never decides a
    /// lifecycle itself.
    kill: KillToolSlot,
    /// The environment inventory query and kill owner (#1369, #1939),
    /// installed by composition after this tool is built. Empty, the
    /// container commands are refused: this tool composes no use case.
    environments: super::agent_cmd_containers::EnvironmentControlSlot,
    /// The container-config listing `get_container_configs` serves (#2024
    /// S4c), composed over the launching agent's checkout beside the spawn
    /// tool's selection. Empty, the command reports the capability
    /// unavailable: this tool composes no use case.
    container_config_roster:
        Option<std::sync::Arc<crate::application::environments::use_cases::ListContainerConfigs>>,
    /// What an ended child left (#2192), installed by composition: served
    /// for `get_state` / `get_messages` once the child can no longer answer.
    ended: super::agent_cmd_ended::EndedChildSlot,
}

/// Where the composed `agent_cmd kill` owner lives (#1936): filled once by
/// the interface after the agent-control tools are built, read by every
/// `kill` command. A second install is ignored: one owner per harness.
#[derive(Clone, Default)]
pub struct KillToolSlot(std::sync::Arc<std::sync::OnceLock<std::sync::Arc<dyn Tool>>>);

impl KillToolSlot {
    /// Install the owner; `true` when this call filled the slot.
    pub fn install(&self, tool: std::sync::Arc<dyn Tool>) -> bool {
        self.0.set(tool).is_ok()
    }

    pub fn get(&self) -> Option<std::sync::Arc<dyn Tool>> {
        self.0.get().cloned()
    }
}

impl std::fmt::Debug for AgentCmdTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentCmdTool")
            .field("kill", &self.kill.get().is_some())
            .finish_non_exhaustive()
    }
}

impl AgentCmdTool {
    /// Create a new `AgentCmdTool` backed by the given registry.
    pub fn new(registry: SubagentRegistry) -> Self {
        Self {
            registry,
            kill: KillToolSlot::default(),
            environments: super::agent_cmd_containers::EnvironmentControlSlot::default(),
            container_config_roster: None,
            ended: super::agent_cmd_ended::EndedChildSlot::default(),
        }
    }

    /// Read the ended-child inspection from a slot composition fills later.
    pub fn with_ended_child_slot(mut self, slot: super::agent_cmd_ended::EndedChildSlot) -> Self {
        self.ended = slot;
        self
    }

    /// Attach composition's container-config listing (#2024 S4c).
    pub fn with_container_config_roster(
        mut self,
        roster: Option<
            std::sync::Arc<crate::application::environments::use_cases::ListContainerConfigs>,
        >,
    ) -> Self {
        self.container_config_roster = roster;
        self
    }

    /// Attach the composed environment control (inventory query and kill
    /// owner) directly: fixtures that compose their own.
    pub fn with_environment_control(
        self,
        control: super::agent_cmd_containers::EnvironmentControl,
    ) -> Self {
        let installed = self.environments.install(control);
        debug_assert!(
            installed,
            "the environment control is composed once per tool"
        );
        self
    }

    /// Read the environment control from a slot shared with whoever fills
    /// it later (composition, once the tool is already registered).
    pub fn with_environment_control_slot(
        mut self,
        slot: super::agent_cmd_containers::EnvironmentControlSlot,
    ) -> Self {
        self.environments = slot;
        self
    }

    /// Attach the composed selected-termination tool that owns `kill`.
    pub fn with_kill_tool(self, kill: std::sync::Arc<dyn Tool>) -> Self {
        self.kill.install(kill);
        self
    }

    /// Read `kill` from a slot shared with whoever fills it later
    /// (composition, once the tool is already registered).
    pub fn with_kill_slot(mut self, slot: KillToolSlot) -> Self {
        self.kill = slot;
        self
    }

    /// The slot the composed `kill` owner is installed into.
    pub fn kill_slot(&self) -> KillToolSlot {
        self.kill.clone()
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
            if args.get("agent_id").and_then(|v| v.as_str()) != Some("*") {
                return Some(ToolResult {
                    content: "agent_cmd error: get_subagents_all requires agent_id \"*\"".into(),
                    is_error: true,
                    image_blocks: vec![],
                    delivery_metadata: None,
                });
            }
            return Some(self.list_all_subagents(args));
        }
        None
    }

    /// Delegate the `kill` command to its owner (#1936): the composed
    /// selected-termination tool parses, invokes and presents; this tool
    /// only routes the command there.
    async fn try_kill_command(
        &self,
        arguments: &str,
        args: &serde_json::Value,
    ) -> Option<ToolResult> {
        let command = args.get("command").and_then(|v| v.as_str())?;
        if command != "kill" {
            return None;
        }
        let target = args.get("agent_id").and_then(|v| v.as_str()).unwrap_or("");
        if let Err(refused) = super::agent_cmd_parse::admit_target(command, target) {
            return Some(ToolResult {
                content: format!("agent_cmd error: {refused}"),
                is_error: true,
                image_blocks: vec![],
                delivery_metadata: None,
            });
        }
        let Some(kill) = self.kill.get() else {
            return Some(ToolResult {
                content: "agent_cmd error: kill is not available in this composition".into(),
                is_error: true,
                image_blocks: vec![],
                delivery_metadata: None,
            });
        };
        Some(
            kill.execute(arguments)
                .await
                .unwrap_or_else(|e| ToolResult {
                    content: format!("agent_cmd error: {e}"),
                    is_error: true,
                    image_blocks: vec![],
                    delivery_metadata: None,
                }),
        )
    }

    /// Queueable forwarded commands carry `"ack":"accept"` — the child acks
    /// ACCEPTANCE promptly (its reader, not the blocked dispatch loop), so the
    /// parent waits only the short interactive timeout, never the 300s
    /// turn-completion deadline (#876/#880).
    fn is_control_command(command: &str) -> bool {
        matches!(
            command,
            "swarm_control"
                | "prompt"
                | "steer"
                | "follow_up"
                | "abort"
                | "set_model"
                | "clear_history"
        )
    }

    /// List every subagent currently tracked by this parent agent's registry.
    fn list_all_subagents(&self, args: &serde_json::Value) -> ToolResult {
        let since = match args.get("since") {
            None | Some(serde_json::Value::Null) => None,
            Some(v) => match v.as_u64() {
                Some(s) => Some(s),
                None => {
                    return ToolResult {
                        content: "agent_cmd error: invalid since cursor".into(),
                        is_error: true,
                        image_blocks: vec![],
                        delivery_metadata: None,
                    };
                }
            },
        };
        let roster = super::subagent_compact_roster::build_compact_subagent_roster(
            &Some(self.registry.clone()),
            since,
        );
        let content = match roster {
            Ok(roster) => serde_json::to_string(&roster).unwrap_or_else(|_| "{}".to_string()),
            Err(e) => {
                return ToolResult {
                    content: format!("agent_cmd error: {e}"),
                    is_error: true,
                    image_blocks: vec![],
                    delivery_metadata: None,
                };
            }
        };
        ToolResult {
            content,
            is_error: false,
            image_blocks: vec![],
            delivery_metadata: None,
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
    DeliveryDecision, PageReport, holds_whole, needs_default_report_backfill, page_report,
    plan_delivery,
};
use full_report::CapRead;

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
        // The report the child's page names, if the supervisor has yet to
        // read it and the page does not hold it whole (#2226).
        let page = match envelope.get("data") {
            Some(data) => page_report(&messages, data),
            None => PageReport::Unnamed,
        };
        let unread_named = match &page {
            PageReport::OffPage(report) if report.is_unread(delivered) => Some(report.clone()),
            _ => None,
        };
        // A first read reads the named report by id instead of paging back.
        // A read that fails falls back to the page path, and the report stays
        // owed: the read never acknowledges past it (review 3). A stand-in
        // for the text is never delivered.
        let mut owed_report = None;
        let named = match (&page, &unread_named) {
            _ if delivered > 0 => false,
            (PageReport::NamesNone | PageReport::OnPage, _) => true,
            (_, Some(report)) => {
                let read = self
                    .read_named_report(socket_path, routed_target_id, report)
                    .await;
                read.map(|text| report.place(&mut messages, text))
                    .map_err(|_| owed_report = Some(report.clone()))
                    .is_ok()
            }
            _ => false,
        };
        let mut older_unread_skipped = None;
        // A child naming no report (`report: null`) has given no
        // substantive reply: a first read delivers the window it holds
        // instead of nothing (#2246).
        let mut report_found = match (delivered, &page) {
            (0, PageReport::NamesNone) => false,
            (_, PageReport::Unnamed | PageReport::NamesNone)
            | (_, PageReport::OnPage | PageReport::OffPage(_)) => true,
        };
        // Only a page that says older history exists is paged back from
        // (#2218); one without it holds the whole transcript.
        while !named
            && needs_default_report_backfill(
                &messages,
                delivered,
                envelope.pointer("/data/hasMoreBefore") == Some(&serde_json::Value::Bool(true)),
            )
        {
            if backfill_pages >= MAX_DEFAULT_REPORT_BACKFILL_PAGES {
                let cap = self
                    .at_backfill_cap(
                        socket_path,
                        routed_target_id,
                        &mut messages,
                        CapRead {
                            delivered,
                            before: envelope.pointer("/data/before").cloned(),
                            named: unread_named.as_ref(),
                            owed: owed_report.as_ref(),
                        },
                    )
                    .await;
                backfill_complete = cap.complete;
                report_found = cap.report_found;
                older_unread_skipped = cap.skipped;
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
        // A named report the first read could not read stays owed until it
        // is held whole: paging that reached only its stub, or not at all,
        // leaves the read incomplete (#2226 review 4).
        match &owed_report {
            Some(report) if holds_whole(&messages, &report.id) => {}
            Some(_) => backfill_complete = false,
            None => {}
        }
        if let Some(data) = envelope.get_mut("data") {
            data["messages"] = serde_json::Value::Array(messages);
            if !backfill_complete {
                data["reportIncomplete"] = serde_json::json!(true);
            }
            if let Some(skipped) = older_unread_skipped {
                data["olderUnreadSkipped"] = skipped;
            }
            if !report_found {
                data["reportFound"] = serde_json::json!(false);
            }
        }
        envelope.to_string()
    }

    fn shape_default_get_messages_report_with_metadata(
        &self,
        agent_id: &str,
        response: &str,
    ) -> (String, Option<String>) {
        super::agent_cmd_report::shape_default_report(&self.registry, agent_id, response)
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
        let receipt = result.delivery_metadata.as_deref();
        let Some(key) = super::agent_cmd_report::report_row_key(&entries, agent_id, receipt) else {
            return;
        };
        let Some(entry) = entries.get_mut(&key) else {
            return;
        };
        let explicit_page = !args.get("count").is_none_or(|v| v.is_null())
            || !args.get("before").is_none_or(|v| v.is_null());
        match plan_delivery(
            command,
            explicit_page,
            result.is_error,
            &result.content,
            result.delivery_metadata.as_deref(),
            &entry.pending_message_reports,
        ) {
            DeliveryDecision::Ignore => {}
            DeliveryDecision::Clear => {
                entry.delivered_message_ordinal = None;
                entry.pending_message_ordinal = None;
                entry.pending_message_reports.clear();
            }
            DeliveryDecision::Acknowledge(index) => {
                if let Some(pending) = entry.pending_message_reports.remove(index) {
                    entry.delivered_message_ordinal = Some(
                        entry
                            .delivered_message_ordinal
                            .unwrap_or(0)
                            .max(pending.ordinal),
                    );
                }
                entry.pending_message_ordinal = entry
                    .pending_message_reports
                    .back()
                    .map(|pending| pending.ordinal);
            }
        }
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "agent_cmd".into(),
            description: "Send commands to a spawned subagent, or manage spawned containers.\n\nAfter a spawned child completes, do not treat the passive note as the answer. Call get_messages with the spawn-returned UUID as agent_id and no count/before to read the default unread report. Do not poll or wait-loop; use get_state only for occasional live supervision.\n\nUse prompt/steer/follow_up with message; abort interrupts; kill terminates. get_subagents_all, get_containers, get_container_configs, and kill_container use agent_id \"*\". get_container_configs lists the container configs spawn can select for this checkout (name, default, source overlay|global, repository); get_containers lists what {\"mode\":\"existing\"} can join (running, empty, retained) plus your session's own not stopped, in ref order, capped with counts; all:true also lists stopped ones and other sessions' that cannot be joined."
                .into(),
            parameters_schema: r#"{"type":"object","properties":{"agent_id":{"type":"string","description":"Spawn-returned subagent UUID; use \"*\" for inventory/container commands."},"command":{"type":"string","enum":["swarm_control","prompt","steer","follow_up","abort","kill","get_state","get_report","get_messages","get_session_stats","get_subagents","get_subagents_all","get_containers","get_container_configs","kill_container","set_model","set_effort","clear_history"],"description":"Command. For completed spawned work, use get_messages without count/before."},"export_raw":{"type":"boolean","description":"With get_report, write retained raw history and spills to artifacts."},"action":{"type":"string","enum":["pause","resume","close","extend","status","usage_budget"]},"reason":{"type":"string"},"deadline_seconds":{"type":"integer","minimum":1,"description":"swarm_control extend: seconds added to the run deadline before resuming a budget-exhausted run."},"token_limit":{"type":["integer","null"],"minimum":1},"strict_unknown":{"type":"boolean"},"message":{"type":"string","description":"Text for prompt, steer, follow_up."},"count":{"type":"integer","description":"Explicit get_messages history page size; cursor-neutral. Omit/null count and before for the unread report."},"before":{"type":"string","description":"Explicit get_messages older page cursor from a prior before field; cursor-neutral."},"since":{"type":"integer","description":"Generation cursor for get_state/get_subagents; unchanged returns only metadata, plus get_state modelTurn while the model runs."},"model":{"type":"string","description":"set_model as provider/model; alternative to provider+model_id."},"provider":{"type":"string","description":"set_model provider; use with model_id."},"model_id":{"type":"string","description":"set_model id; use with provider."},"effort":{"type":"string","enum":["none","low","medium","high","xhigh","max"],"description":"set_effort value."},"all":{"type":"boolean","description":"get_containers: also list stopped ones and other sessions' that cannot be joined."},"ref":{"type":"string","description":"Container ref for kill_container, e.g. C1."},"name":{"type":"string","description":"Container name for kill_container; alternative to ref."}},"required":["agent_id","command"]}"#
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
                    let control = self.environments.get();
                    return Ok(super::agent_cmd_containers::execute_container_command(
                        control.as_ref().map(|control| &control.list),
                        control.as_ref().map(|control| &control.kill),
                        self.container_config_roster.as_ref(),
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
                if let Some(result) = self.try_kill_command(&args, value).await {
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
                            delivery_metadata: None,
                        });
                    }
                },
                Err(e) => {
                    return Ok(ToolResult {
                        content: format!("agent_cmd error: invalid JSON: {e}"),
                        is_error: true,
                        image_blocks: vec![],
                        delivery_metadata: None,
                    });
                }
            };

            let default_get_messages_report = parsed.as_ref().ok().is_some_and(|value| {
                value.get("command").and_then(|v| v.as_str()) == Some("get_messages")
                    && value.get("count").is_none_or(|v| v.is_null())
                    && value.get("before").is_none_or(|v| v.is_null())
            });

            let route = if command == "swarm_control" {
                super::subagent_routing::resolve_inspection_route(&self.registry, &agent_id)
            } else if let Some(routable) =
                super::subagent_routing::RoutableInspectionCommand::from_agent_cmd(&command)
            {
                debug_assert!(matches!(
                    routable,
                    super::subagent_routing::RoutableInspectionCommand::GetReport
                        | super::subagent_routing::RoutableInspectionCommand::GetMessages
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
                    // An ended child answers from what it left (#2192).
                    let arguments = parsed.as_ref().ok().cloned().unwrap_or_default();
                    let ended = super::agent_cmd_ended::answer_if_ended(
                        (&self.registry, self.ended.get()),
                        (&agent_id, &command, &arguments),
                    );
                    if let Some(result) = ended.await {
                        return Ok(result);
                    }
                    return Ok(ToolResult {
                        content: format!("agent_cmd error: {e}"),
                        is_error: true,
                        image_blocks: vec![],
                        delivery_metadata: None,
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
            // Inspection is bounded independently of acceptance-ack semantics.
            let send =
                if matches!(command.as_str(), "get_state") || Self::is_control_command(&command) {
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
            // monitor events; the transport ack alone cannot prove task progress
            // and must not race with `agent_end` by marking the child Busy here.
            match send {
                Ok(response) => {
                    let rejected = serde_json::from_str::<serde_json::Value>(&response)
                        .is_ok_and(|value| value["success"] == false);
                    let routed = routed_target_id.as_deref();
                    let response = if default_get_messages_report {
                        let backfilled = self
                            .expand_default_get_messages_response(
                                socket_path,
                                routed,
                                &response,
                                &agent_id,
                            )
                            .await;
                        self.expand_collapsed_final_report(
                            socket_path,
                            routed,
                            backfilled,
                            &agent_id,
                        )
                        .await
                    } else if command == "get_report" {
                        self.expand_report(socket_path, routed, response).await
                    } else {
                        response
                    };
                    let (content, delivery_metadata) = if default_get_messages_report {
                        self.shape_default_get_messages_report_with_metadata(&agent_id, &response)
                    } else {
                        (response, None)
                    };
                    Ok(ToolResult {
                        content,
                        is_error: rejected,
                        image_blocks: vec![],
                        delivery_metadata,
                    })
                }
                Err(e) => Ok(ToolResult {
                    content: format!("agent_cmd error: {e}"),
                    is_error: true,
                    image_blocks: vec![],
                    delivery_metadata: None,
                }),
            }
        })
    }
}

#[cfg(test)]
#[path = "agent_cmd_answerless_report_tests.rs"]
mod answerless_report_tests;
#[cfg(test)]
#[path = "agent_cmd_definition_tests.rs"]
mod definition_tests;
#[cfg(test)]
#[path = "agent_cmd_delivery_tests.rs"]
mod delivery_tests;
#[path = "agent_cmd_full_report.rs"]
mod full_report;
#[cfg(test)]
#[path = "agent_cmd_full_report_tests.rs"]
mod full_report_tests;
#[cfg(test)]
#[path = "agent_cmd_get_subagents_all_tests.rs"]
mod get_subagents_all_tests;
#[cfg(test)]
#[path = "agent_cmd_named_report_tests.rs"]
mod named_report_tests;
#[cfg(test)]
#[path = "agent_cmd_nudge_report_tests.rs"]
mod nudge_report_tests;
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

#[cfg(test)]
#[path = "agent_cmd_timeout_tests.rs"]
mod timeout_tests;
