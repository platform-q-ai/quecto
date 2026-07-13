use super::*;

impl App {
    pub(super) fn handle_event(&mut self, event: Event) {
        match event {
            Event::AgentStart => self.handle_agent_start(),
            Event::Token { token } => self.master_session.chat.append_token(&token),
            Event::TurnStart => {}
            Event::TurnEnd { message } => self.handle_turn_end(message),
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
            Event::AgentEnd { message_refs, .. } if self.agent_state.end() => {
                self.maybe_recover_from_refs(&message_refs);
                self.handle_agent_end();
            }
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
        self.tools_this_turn = 0;
        self.open_tool_calls = 0;
        self.active_turn_start = self.master_session.chat.entry_count();
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
        // Flush onto the currently-viewed session (the same place
        // `handle_subagent_notification` deferred them), so a coalesced summary is
        // visible whether the master or a sub-agent is selected.
        let session = self.active_session_mut();
        Self::flush_deferred_notes(&mut session.chat, &mut session.deferred_subagent_notes);
    }

    fn handle_turn_end(&mut self, message: serde_json::Value) {
        self.master_session.chat.finalize_assistant();
        // #1060: messageRefs identify the turn; fetch-on-miss only when the
        // active-turn stream did not already deliver full content.
        let refs = message_refs_from_value(&message);
        let content_len = message.get("contentLength").and_then(|v| v.as_u64());
        self.maybe_recover_from_refs_with_len(&refs, content_len);
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

    /// #1060 recovery: common streamed path = zero fetches; miss = get_message
    /// per missing ref with request-id gating. Never blindly append-all (#1075).
    ///
    /// Both `turn_end` and `agent_end` may carry the same refs; skip message ids
    /// that already have an in-flight recovery request so we never double-fetch.
    fn maybe_recover_from_refs(&mut self, refs: &[String]) {
        self.maybe_recover_from_refs_with_len(refs, None);
    }

    fn maybe_recover_from_refs_with_len(
        &mut self,
        refs: &[String],
        expected_content_len: Option<u64>,
    ) {
        if refs.is_empty() {
            return;
        }
        // A tool call whose end-event never arrived (e.g. a dropped
        // ToolExecutionEnd on the bounded progress channel) leaves its box
        // unresolved even when ref cardinality + contentLength look complete;
        // force recovery so the missing result is fetched (#1060 review 3).
        let force = self.open_tool_calls > 0;
        if !force
            && !self.needs_message_recovery_for(
                refs,
                &self.latest_assistant_text(),
                self.tools_this_turn,
                expected_content_len,
            )
        {
            return;
        }
        // turn_end and agent_end may carry the same refs. The first event owns
        // the batch; do not create an unfillable duplicate with zero requests.
        if refs.iter().any(|message_id| {
            self.pending_message_recovery
                .values()
                .any(|pending| pending.message_id == *message_id)
        }) {
            return;
        }
        let batch_id = format!("recovery-batch-{}", uuid_like());
        let target_end = self.master_session.chat.entry_count();
        self.message_recovery_batches.insert(
            batch_id.clone(),
            MessageRecoveryBatch {
                refs: refs.to_vec(),
                responses: std::collections::HashMap::new(),
                target_start: self.active_turn_start.min(target_end),
                target_end,
                agent_id: None,
            },
        );
        for message_id in refs {
            if self
                .pending_message_recovery
                .values()
                .any(|pending| pending.message_id == *message_id)
            {
                continue;
            }
            let req_id = format!("msg-recovery-{}", uuid_like());
            self.pending_message_recovery.insert(
                req_id.clone(),
                PendingMessageRecovery {
                    message_id: message_id.clone(),
                    batch_id: batch_id.clone(),
                },
            );
            self.send_command(Command::GetMessage {
                id: Some(req_id),
                message_id: message_id.clone(),
                agent_id: None,
            });
        }
    }

    /// Conservative completeness check. A text-only turn can be proven complete
    /// from contentLength. A multi-message/tool turn cannot: event delivery has
    /// no stable IDs, so fetch refs and reconcile the exact turn range. This
    /// avoids treating "one of two tools observed" as complete.
    pub(super) fn needs_message_recovery_for(
        &self,
        refs: &[String],
        assistant_text: &str,
        tools_this_turn: usize,
        expected_content_len: Option<u64>,
    ) -> bool {
        if refs.is_empty() {
            return false;
        }
        let trimmed = assistant_text.trim();
        if trimmed.is_empty() || trimmed == "…" || trimmed == "..." {
            return true;
        }
        if let Some(expected) = expected_content_len
            && (assistant_text.len() as u64) < expected
        {
            return true;
        }
        // A fully observed tool call contributes one assistant tool-call
        // message and one tool-result message; the final assistant contributes
        // one more. Any other cardinality proves at least one role was missed.
        let expected_refs = tools_this_turn.saturating_mul(2).saturating_add(1);
        refs.len() != expected_refs
    }

    fn latest_assistant_text(&self) -> String {
        self.master_session
            .chat
            .entries()
            .iter()
            .rev()
            .find_map(|e| match e {
                crate::interface::components::chat::ChatEntry::Assistant { text, .. } => {
                    Some(text.clone())
                }
                _ => None,
            })
            .unwrap_or_default()
    }

    /// Buffer a gated response and atomically replace the original turn once all
    /// refs arrive. Unknown, stale, failed, or mismatched responses never mutate
    /// chat state.
    pub(super) fn handle_get_message_recovery(
        &mut self,
        id: Option<&str>,
        success: bool,
        data: Option<serde_json::Value>,
    ) {
        let Some(req_id) = id else { return };
        let Some(pending) = self.pending_message_recovery.remove(req_id) else {
            return;
        };
        if !success {
            // A ref that cannot be fetched (unknown/cancelled id) can never
            // complete this batch; abandon it so it does not linger unfillable
            // and leak, and drop the already-fetched siblings with it (#1060
            // review). The turn stays as-streamed.
            self.abandon_recovery_batch(&pending.batch_id);
            return;
        }
        let Some(data) = data else {
            self.abandon_recovery_batch(&pending.batch_id);
            return;
        };
        if data.get("id").and_then(|v| v.as_str()) != Some(pending.message_id.as_str()) {
            self.abandon_recovery_batch(&pending.batch_id);
            return;
        }
        let Some(batch) = self.message_recovery_batches.get_mut(&pending.batch_id) else {
            return;
        };
        batch.responses.insert(pending.message_id, data);
        if batch.responses.len() != batch.refs.len() {
            return;
        }
        let batch = self
            .message_recovery_batches
            .remove(&pending.batch_id)
            .unwrap();
        let entries = recovered_chat_entries(&batch.refs, &batch.responses);
        // Apply to the session the batch was created for. Child recovery is
        // routed through the master (agent_id set), so a completed child batch
        // is applied to that child's chat here — not only the master's (#1060
        // review, F1). A child evicted mid-recovery drops silently.
        match &batch.agent_id {
            None => {
                self.master_session.chat.replace_range(
                    batch.target_start,
                    batch.target_end,
                    entries,
                );
            }
            Some(child) => {
                if let Some(session) = self.subagents.sessions.get_mut(child) {
                    session
                        .chat
                        .replace_range(batch.target_start, batch.target_end, entries);
                }
            }
        }
    }

    /// Drop a recovery batch and any still-pending sibling requests that point at
    /// it. Called when one ref of the batch cannot be resolved, so a partially
    /// filled batch does not linger unfillable in `message_recovery_batches`
    /// (unbounded growth under repeated failures) (#1060 review).
    pub(super) fn abandon_recovery_batch(&mut self, batch_id: &str) {
        self.message_recovery_batches.remove(batch_id);
        self.pending_message_recovery
            .retain(|_, pending| pending.batch_id != batch_id);
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
            self.subagents.awaited_agent_id = args
                .get("agent_id")
                .and_then(|v| v.as_str())
                .map(str::to_string);
        }
        let is_spawn = tool_name == "spawn";
        // Every model-issued tool call — even one whose box is suppressed (spawn)
        // — appends a tool-call + tool-result message pair to the conversation
        // ledger and therefore contributes to end-of-turn `messageRefs`. Count it
        // regardless of display suppression, or `needs_message_recovery_for`
        // undercounts on spawn turns and fires needless recovery (#1060 review).
        self.tools_this_turn = self.tools_this_turn.saturating_add(1);
        self.open_tool_calls = self.open_tool_calls.saturating_add(1);
        if suppress_tool_box(&tool_name, &args) {
            self.master_session.chat.finalize_assistant();
        } else {
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
        // (e.g. "Sub-agent 'poet-2' ended a turn (status: idle). Inspect agent_cmd get_messages
        // when you need its output."), so we do NOT re-prefix the agent id here —
        // that just duplicated the name.
        let message = crate::interface::ansi::sanitize_control(&message);
        // Never split an in-flight streaming response: if the parent is mid-turn,
        // defer the note and flush it when the parent goes idle (handle_agent_end).
        // Shared defer/flush policy with the per-session path (#828).
        // Surface the note on the CURRENTLY-VIEWED session so it is visible even
        // when a sub-agent is selected (the master chat is not rendered then) —
        // sub-agent completion notes are operator-facing status, not part of any
        // one transcript. When the master is active this is `master_session`, so
        // the existing master-path behaviour is unchanged.
        let running = self.agent_state.is_running();
        let session = self.active_session_mut();
        Self::push_or_defer_note(
            &mut session.chat,
            &mut session.deferred_subagent_notes,
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
        let tracked = &self.subagents.tracked;
        if tracked.get(&sanitized).is_some_and(|e| !e.optimistic) {
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
            read_only: spawn_args_are_read_only(args),
        });
        // Mark this as an unconfirmed local guess: the kernel has not registered
        // the child yet, so a snapshot taken in that window must not evict it
        // (#866). Cleared once a payload confirms the agent.
        tracked.optimistic = true;
        self.subagents.tracked.insert(sanitized, tracked);
    }

    fn handle_tool_end(
        &mut self,
        tool_call_id: String,
        tool_name: String,
        result: serde_json::Value,
        is_error: bool,
    ) {
        let result_text = crate::infrastructure::client::extract_result_text(&result);
        self.open_tool_calls = self.open_tool_calls.saturating_sub(1);
        self.master_session
            .chat
            .complete_tool(&tool_call_id, &result_text, is_error, None);
        if tool_name == "spawn" && !is_error {
            self.mark_spawned_subagent_running(&result_text);
        }
        if is_subagent_tool(&tool_name) {
            self.send_command(Command::GetSubagents { id: None });
        }
        if self.subagents.awaited_agent_id.is_some() {
            self.subagents.awaited_agent_id = None;
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
        if let Some(entry) = self.subagents.tracked.get_mut(&sanitized) {
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
        // Preserve the live auto-continue/nudge state across the rebuild so the
        // compact line doesn't reset to `auto:off` on every event (#897 AC2).
        self.mirror_automation_to_bar();
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

fn spawn_args_are_read_only(args: &serde_json::Value) -> bool {
    args.get("read_only")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || args
            .get("disable_tools")
            .and_then(|v| v.as_array())
            .is_some_and(|tools| {
                let has = |name| tools.iter().any(|v| v.as_str() == Some(name));
                has("write") && has("edit")
            })
}

pub(super) fn recovered_chat_entries(
    refs: &[String],
    responses: &std::collections::HashMap<String, serde_json::Value>,
) -> Vec<crate::interface::components::chat::ChatEntry> {
    use crate::interface::components::chat::ChatEntry;
    let mut entries = Vec::new();
    let mut tools = std::collections::HashMap::<String, usize>::new();
    // Tool calls whose box is suppressed in the live transcript (spawn) must stay
    // suppressed here too, or `replace_range` re-materializes a box the stream
    // intentionally hid (#1060 review). Track their ids so the matching
    // tool-result message is dropped as well.
    let mut suppressed_calls = std::collections::HashSet::<String>::new();
    for message_id in refs {
        let Some(data) = responses.get(message_id) else {
            continue;
        };
        let role = data.get("role").and_then(|v| v.as_str()).unwrap_or("");
        let content = data.get("content").and_then(|v| v.as_str()).unwrap_or("");
        match role {
            "assistant" => {
                if let Some(calls) = data.get("toolCalls").and_then(|v| v.as_array()) {
                    for call in calls {
                        let id = call
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        if id.is_empty() {
                            continue;
                        }
                        let name = call
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("tool")
                            .to_string();
                        if suppress_tool_box(&name, &serde_json::Value::Null) {
                            suppressed_calls.insert(id);
                            continue;
                        }
                        let args = call
                            .get("arguments")
                            .map(|v| {
                                v.as_str()
                                    .map(str::to_string)
                                    .unwrap_or_else(|| v.to_string())
                            })
                            .unwrap_or_else(|| "{}".into());
                        tools.insert(id.clone(), entries.len());
                        entries.push(ChatEntry::ToolExecution {
                            tool_call_id: id,
                            tool_name: name,
                            parsed_args: serde_json::from_str(&args).ok(),
                            args,
                            result: None,
                            is_error: false,
                            duration_ms: None,
                        });
                    }
                }
                if !content.is_empty() {
                    entries.push(ChatEntry::Assistant {
                        text: content.to_string(),
                        streaming: false,
                    });
                }
            }
            "tool" => {
                let call_id = data
                    .get("toolCallId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if suppressed_calls.contains(&call_id) {
                    continue;
                }
                let is_error = data
                    .get("isError")
                    .or_else(|| data.get("is_error"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if let Some(index) = tools.get(&call_id).copied() {
                    if let Some(ChatEntry::ToolExecution {
                        result,
                        is_error: entry_error,
                        ..
                    }) = entries.get_mut(index)
                    {
                        *result = Some(content.to_string());
                        *entry_error = is_error;
                    }
                } else if !call_id.is_empty() {
                    entries.push(ChatEntry::ToolExecution {
                        tool_call_id: call_id,
                        tool_name: data
                            .get("toolName")
                            .and_then(|v| v.as_str())
                            .unwrap_or("tool")
                            .to_string(),
                        args: String::new(),
                        parsed_args: None,
                        result: Some(content.to_string()),
                        is_error,
                        duration_ms: None,
                    });
                }
            }
            _ => {}
        }
    }
    entries
}

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
#[path = "app_events_test_support.rs"]
mod app_events_test_support;
#[cfg(test)]
#[path = "app_events_cursor_tests.rs"]
mod cursor_tests;
#[cfg(test)]
#[path = "app_events_readonly_tests.rs"]
mod readonly_tests;

fn message_refs_from_value(v: &serde_json::Value) -> Vec<String> {
    let candidates = [v.get("messageRefs"), v.get("message_refs")];
    for c in candidates.into_iter().flatten() {
        if let Some(arr) = c.as_array() {
            let refs: Vec<String> = arr
                .iter()
                .filter_map(|item| {
                    if let Some(s) = item.as_str() {
                        return Some(s.to_string());
                    }
                    item.get("id")
                        .and_then(|id| id.as_str())
                        .map(str::to_string)
                })
                .filter(|s| !s.is_empty())
                .collect();
            if !refs.is_empty() {
                return refs;
            }
        }
    }
    Vec::new()
}

/// A process-unique token for request/batch ids. Combines a wall-clock stamp
/// (readability in logs) with a monotonic counter so two calls in the same
/// nanosecond — `SystemTime` is not guaranteed strictly increasing — cannot
/// collide and clobber each other's pending-recovery entry (#1060 review).
pub(super) fn uuid_like() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{nanos:x}-{seq:x}")
}

#[cfg(test)]
#[path = "app_events_tests.rs"]
mod tests;
