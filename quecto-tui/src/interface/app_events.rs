use super::*;

impl App {
    pub(super) fn handle_event(&mut self, event: Event) {
        match event {
            Event::AgentStart => self.handle_agent_start(),
            Event::Token { token } => self.master_session.chat.append_token(&token),
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
            Event::SubagentNotification {
                agent_id, message, ..
            } => self.handle_subagent_notification(agent_id, message),
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
        // Mirror the abort-aware run state onto the master session's `running`
        // flag so the unified working indicator is driven by one per-session
        // flag for master and sub-agents alike (#828).
        self.master_session.running = true;
        self.master_session.footer.set_streaming(true);
        self.spinner = Some(Spinner::new("Working... (Esc to interrupt)"));
    }

    fn handle_agent_end(&mut self) {
        self.master_session.running = false;
        self.master_session.footer.set_streaming(false);
        self.spinner = None;
        self.master_session.chat.finalize_assistant();
        // Parent is now idle — flush any sub-agent completion notes that arrived
        // mid-turn, so they appear after the finished response instead of in it.
        Self::flush_deferred_notes(
            &mut self.master_session.chat,
            &mut self.master_session.deferred_subagent_notes,
        );
    }

    fn handle_turn_end(&mut self, message: serde_json::Value) {
        self.master_session.chat.finalize_assistant();
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
            self.master_session
                .footer
                .update_context_usage(used, window);
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
        }
        let is_spawn = tool_name == "spawn";
        if !suppress_tool_box(&tool_name, &args) {
            self.master_session
                .chat
                .start_tool(tool_call_id, tool_name, args_str);
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

    /// Render a passive one-line sub-agent completion note (#816) as a single
    /// chat status line. It must NOT steal focus, open the inspector panel, or
    /// require interaction — it is purely informational. Both the agent id and
    /// the message are sanitized for terminal-control sequences before display.
    fn handle_subagent_notification(&mut self, _agent_id: String, message: String) {
        // The message is already a concise, self-naming one-liner from the kernel
        // (e.g. "Agent 'poet-2' completed and is ready for inspection"), so we do
        // NOT re-prefix the agent id here — that just duplicated the name.
        let message = crate::interface::ansi::sanitize_control(&message);
        // Never split an in-flight streaming response: if the parent is mid-turn,
        // defer the note and flush it when the parent goes idle (handle_agent_end).
        // Shared defer/flush policy with the per-session path (#828).
        let running = self.agent_state.is_running();
        Self::push_or_defer_note(
            &mut self.master_session.chat,
            &mut self.master_session.deferred_subagent_notes,
            running,
            message,
        );
    }

    pub(super) fn track_starting_subagent(&mut self, args: &serde_json::Value) {
        let Some(agent_id) = args.get("agent_id").and_then(|v| v.as_str()) else {
            return;
        };
        let sanitized = crate::interface::ansi::sanitize_control(agent_id);
        // If the kernel already confirmed this id (a non-optimistic entry), do
        // not clobber it with a fresh unconfirmed "starting" guess. A re-played
        // or duplicate spawn ToolStart (event replay on reconnect) would
        // otherwise reset started_at/status and re-mark it optimistic, partly
        // re-opening the #831 drop path for the grace window (review).
        if self
            .subagent_local
            .get(&sanitized)
            .is_some_and(|e| !e.optimistic)
        {
            return;
        }
        let mut tracked = TrackedSubagent::new(crate::infrastructure::client::SubagentInfoEvent {
            agent_id: sanitized.clone(),
            status: "starting".to_string(),
            last_tool: None,
            last_error: None,
            pid: 0,
            socket_path: None,
            parent_id: None,
            workflow: None,
        });
        // Mark this as an unconfirmed local guess: the kernel has not registered
        // the child yet, so a snapshot taken in that window must not evict it
        // (#866). Cleared once a payload confirms the agent.
        tracked.optimistic = true;
        self.subagent_local.insert(sanitized, tracked);
    }

    fn handle_tool_end(
        &mut self,
        tool_call_id: String,
        tool_name: String,
        result: serde_json::Value,
        is_error: bool,
    ) {
        let result_text = crate::infrastructure::client::extract_result_text(&result);
        self.master_session
            .chat
            .complete_tool(&tool_call_id, &result_text, is_error, None);
        if tool_name == "spawn" && !is_error {
            self.mark_spawned_subagent_running(&result_text);
        }
        if is_subagent_tool(&tool_name) {
            self.send_command(Command::GetSubagents { id: None });
        }
        if self.awaited_agent_id.is_some() {
            self.awaited_agent_id = None;
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
        self.master_session.workflow_bar = build_workflow_state(
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

/// Whether a tool's chat box should be hidden. Only `spawn` is suppressed —
/// its effect is shown in the sub-agent panel / status bar instead. Every
/// model-issued `agent_cmd` command renders a normal tool box (#871), including
/// the read-only queries (#865) and the control/destructive commands
/// (`prompt`/`steer`/`abort`/`kill`/…), so the transcript stays complete and it
/// is clear why a sub-agent stopped. The TUI's OWN internal `get_state`/stats
/// polling flows through `app_response.rs` (Response events), not this path, so
/// it stays box-free regardless.
pub(super) fn suppress_tool_box(tool_name: &str, _args: &serde_json::Value) -> bool {
    // #871: every model-issued `agent_cmd` invocation renders as a normal tool
    // call, including the control/destructive commands (`prompt`/`steer`/
    // `abort`/`kill`) that used to be hidden. Hiding them left the transcript
    // incomplete and made it hard to see why a sub-agent stopped. Only `spawn`
    // stays suppressed — the sub-agent status bar/panel shows it instead. The
    // TUI's OWN internal `get_state`/stats polling flows through Response events
    // (`app_response.rs`), not this tool path, so it remains box-free regardless.
    tool_name == "spawn"
}

#[cfg(test)]
#[path = "app_events_tests.rs"]
mod tests;
