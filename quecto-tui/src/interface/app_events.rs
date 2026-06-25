use super::*;

impl App {
    pub(super) fn handle_event(&mut self, event: Event) {
        match event {
            Event::AgentStart => self.handle_agent_start(),
            Event::Token { token } => self.chat.append_token(&token),
            Event::TurnStart => {}
            Event::TurnEnd { message, .. } => self.handle_turn_end(message),
            Event::ToolExecutionStart {
                tool_call_id,
                tool_name,
                args,
            } => self.handle_tool_start(tool_call_id, tool_name, args),
            Event::ToolExecutionEnd {
                tool_call_id,
                tool_name,
                result,
                is_error,
            } => self.handle_tool_end(tool_call_id, tool_name, result, is_error),
            Event::AgentEnd { .. } if self.agent_state.end() => self.handle_agent_end(),
            Event::AgentEnd { .. } => {}
            Event::Response {
                id,
                command,
                success,
                data,
                error,
            } => self.handle_response(id, command, success, data, error),
            Event::SubagentStateChanged { subagents } => self.update_subagent_bar(subagents),
            // Parent-forwarded per-turn appends (#797) are superseded by the
            // direct connect-on-select stream (#800); the active sub-agent's own
            // connection now carries its live content. Ignored here.
            Event::SubagentMessagesAppended { .. } => {}
            Event::WorkflowState {
                agent_id,
                steps,
                progress,
                active_issue,
                mode,
                active_template,
                available_templates,
            } => self.handle_workflow_state(WorkflowStateEvent {
                agent_id,
                steps,
                progress,
                active_issue,
                mode,
                active_template,
                available_templates,
            }),
            _ => {}
        }
    }

    fn handle_agent_start(&mut self) {
        self.agent_state.start();
        self.footer.set_streaming(true);
        self.spinner = Some(Spinner::new("Working... (Esc to interrupt)"));
    }

    fn handle_agent_end(&mut self) {
        self.footer.set_streaming(false);
        self.spinner = None;
        self.chat.finalize_assistant();
    }

    fn handle_turn_end(&mut self, message: serde_json::Value) {
        self.chat.finalize_assistant();
        // `usage` is absent on streaming OpenAI-compatible providers (e.g.
        // Fireworks) because their SSE stream does not carry token counts.
        // Don't gate context-gauge updates on it — `contextTokens` is emitted
        // independently and must still drive the footer.
        let total = message
            .get("usage")
            .and_then(|u| u.get("total"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let context_tokens = message.get("contextTokens").and_then(|v| v.as_u64());
        if let (Some(used), Some(window)) = (
            context_tokens,
            message
                .get("maxContextTokens")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize),
        ) {
            self.footer.update_context_usage(used, window);
            self.context_stats_requested = true;
            // Context came inline, but cost does not ride the turn_end event —
            // refresh session stats (quietly) so the footer cost stays current.
            self.send_session_stats_footer();
        } else if total > 0 && !self.context_stats_requested {
            self.context_stats_requested = true;
            self.send_session_stats_footer();
        }
    }

    fn handle_tool_start(
        &mut self,
        tool_call_id: String,
        tool_name: String,
        args: serde_json::Value,
    ) {
        let args_str = if args.is_object() || args.is_array() {
            serde_json::to_string(&args).unwrap_or_default()
        } else {
            args.to_string()
        };
        self.update_tool_spinner(&tool_name, &args, &args_str);
        // Mark the awaited sub-agent so its row shows a per-row "awaiting"
        // indicator instead of the shared spinner line.
        if tool_name == "agent_cmd" && args.get("command").and_then(|v| v.as_str()) == Some("await")
        {
            self.awaited_agent_id = args
                .get("agent_id")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            self.rebuild_subagent_bar();
        }
        let is_spawn = tool_name == "spawn";
        if !suppress_tool_box(&tool_name, &args) {
            self.chat.start_tool(tool_call_id, tool_name, args_str);
        }
        if is_spawn {
            self.track_starting_subagent(&args);
        }
    }

    fn update_tool_spinner(&mut self, tool_name: &str, args: &serde_json::Value, args_str: &str) {
        let Some(spinner) = &mut self.spinner else {
            return;
        };
        let msg = match tool_name {
            "spawn" => format!("Spawning {}...", sanitized_arg(args, "agent_id", "agent")),
            // For `await`, keep a generic, stable message — the awaited agent is
            // marked on its own sub-agent row, so the shared line stays put.
            "agent_cmd" if args.get("command").and_then(|v| v.as_str()) == Some("await") => {
                "Working... (Esc to interrupt)".to_string()
            }
            "agent_cmd" => format!(
                "{} → {}...",
                sanitized_arg(args, "command", "?"),
                sanitized_arg(args, "agent_id", "?")
            ),
            _ => format!("{} {}...", tool_name, truncate_args(args_str)),
        };
        spinner.set_message(&msg);
    }

    fn track_starting_subagent(&mut self, args: &serde_json::Value) {
        let Some(agent_id) = args.get("agent_id").and_then(|v| v.as_str()) else {
            return;
        };
        let sanitized = crate::interface::ansi::sanitize_control(agent_id);
        self.subagent_local.insert(
            sanitized.clone(),
            TrackedSubagent::new(crate::infrastructure::client::SubagentInfoEvent {
                agent_id: sanitized,
                status: "starting".to_string(),
                last_tool: None,
                last_error: None,
                pid: 0,
                socket_path: None,
                parent_id: None,
                workflow: None,
            }),
        );
        self.rebuild_subagent_bar();
    }

    fn handle_tool_end(
        &mut self,
        tool_call_id: String,
        tool_name: String,
        result: serde_json::Value,
        is_error: bool,
    ) {
        let result_text = crate::infrastructure::client::extract_result_text(&result);
        self.chat
            .complete_tool(&tool_call_id, &result_text, is_error, None);
        if tool_name == "spawn" && !is_error {
            self.mark_spawned_subagent_running(&result_text);
        }
        if is_subagent_tool(&tool_name) {
            self.send_command(Command::GetSubagents { id: None });
        }
        if self.awaited_agent_id.is_some() {
            self.awaited_agent_id = None;
            self.rebuild_subagent_bar();
        }
        if let Some(spinner) = &mut self.spinner {
            spinner.set_message("Working... (Esc to interrupt)");
        }
    }

    fn mark_spawned_subagent_running(&mut self, result_text: &str) {
        let Some(start) = result_text.find('\'') else {
            return;
        };
        let Some(end) = result_text[start + 1..].find('\'') else {
            return;
        };
        let agent_id = &result_text[start + 1..start + 1 + end];
        let sanitized = crate::interface::ansi::sanitize_control(agent_id);
        if let Some(entry) = self.subagent_local.get_mut(&sanitized) {
            entry.info.status = "running".to_string();
        }
        self.rebuild_subagent_bar();
    }

    fn handle_workflow_state(&mut self, workflow: WorkflowStateEvent) {
        let WorkflowStateEvent {
            agent_id,
            steps,
            progress,
            active_issue,
            mode,
            active_template,
            available_templates,
        } = workflow;
        // Only the connected agent's OWN workflow_state updates its bar; events
        // with a different agent_id are descendants' forwarded events (Stage B).
        // Compare to the connected id so a *named*/resumed agent still updates.
        if let Some(id) = agent_id.as_deref() {
            if self.connected_agent_id.as_deref() != Some(id) {
                return;
            }
        }
        self.workflow_bar = build_workflow_state(
            &steps,
            &progress,
            &active_issue,
            &mode,
            &active_template,
            &available_templates,
        );
    }
}

/// Build a `WorkflowBarState` from the parts of a `workflow_state` event. Shared
/// by the master path (`handle_workflow_state`) and the per-sub-agent routing
/// (`route_subagent_event`) so both render an identical bar (#802).
pub(super) fn build_workflow_state(
    steps: &[serde_json::Value],
    progress: &serde_json::Value,
    active_issue: &Option<serde_json::Value>,
    mode: &Option<String>,
    active_template: &Option<serde_json::Value>,
    available_templates: &Option<Vec<serde_json::Value>>,
) -> workflow_bar::WorkflowBarState {
    let mut event = serde_json::json!({
        "steps": steps,
        "progress": progress,
    });
    if let Some(issue) = active_issue {
        event["activeIssue"] = issue.clone();
    }
    if let Some(m) = mode {
        event["mode"] = serde_json::json!(m);
    }
    if let Some(tpl) = active_template {
        event["activeTemplate"] = tpl.clone();
    }
    if let Some(templates) = available_templates {
        event["availableTemplates"] = serde_json::json!(templates);
    }
    workflow_bar::parse_workflow_event(&event)
}

struct WorkflowStateEvent {
    agent_id: Option<String>,
    steps: Vec<serde_json::Value>,
    progress: serde_json::Value,
    active_issue: Option<serde_json::Value>,
    mode: Option<String>,
    active_template: Option<serde_json::Value>,
    available_templates: Option<Vec<serde_json::Value>>,
}

fn sanitized_arg(args: &serde_json::Value, key: &str, fallback: &str) -> String {
    crate::interface::ansi::sanitize_control(
        args.get(key).and_then(|v| v.as_str()).unwrap_or(fallback),
    )
}

/// Whether a tool's chat box should be hidden (its output is shown elsewhere or
/// is noise). `spawn` and `agent_cmd` control/state-query commands are
/// suppressed — the latter dump raw status/workflow JSON already shown in the
/// sub-agent panel, which flashes as the model polls. Content reads
/// (`get_messages*`) and one-shot `await` results still render.
pub(super) fn suppress_tool_box(tool_name: &str, args: &serde_json::Value) -> bool {
    match tool_name {
        "spawn" => true,
        "agent_cmd" => {
            let cmd = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
            const HIDDEN: &[&str] = &[
                "prompt",
                "steer",
                "abort",
                "get_state",
                "get_subagents",
                "get_session_stats",
                "get_extensions",
            ];
            HIDDEN.contains(&cmd)
        }
        _ => false,
    }
}

#[cfg(test)]
#[path = "app_events_tests.rs"]
mod tests;
