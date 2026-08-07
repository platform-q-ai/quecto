use super::*;
use crate::protocol::client::{ToolCatalogueEntry, ToolScope};

fn effective_profile_scope(entry: &ToolCatalogueEntry) -> Option<ToolScope> {
    entry.profile_scope.or_else(|| {
        entry
            .profile_enabled
            .map(super::tool_policy::legacy_profile_enabled_scope)
    })
}

#[cfg(test)]
pub(super) use super::app_message_recovery::recovered_chat_entries;

impl App {
    pub(super) fn handle_event(&mut self, event: Event) {
        match event {
            Event::AgentStart => self.handle_agent_start(),
            Event::Workspace { path } => {
                let root = std::path::PathBuf::from(path);
                if self.workspace.root.as_ref() != Some(&root) {
                    self.workspace.files_autocomplete.invalidate_loaded_files();
                }
                self.workspace.root = Some(root.clone());
                self.workspace.git_repo = Some(root.clone());
                self.master_session.footer.set_pwd_path(&root);
                self.apply_git_branch(app_git::read_git_branch_from(&root));
            }
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
            Event::ToolCatalogueChanged { after, .. } => self.merge_tool_catalogue_event(after),
            Event::ToolPolicyChanged { results, .. } => self.merge_tool_policy_results(results),
            _ => {}
        }
    }

    pub(super) fn merge_tool_catalogue_event(&mut self, entries: Vec<ToolCatalogueEntry>) {
        self.merge_tool_catalogue_entries(entries);
        if self.tool_policy_modal_pending_catalogue_id.is_none() {
            self.open_pending_tool_policy_modal_after_catalogue_update();
        }
    }

    fn merge_tool_catalogue_entries(&mut self, entries: Vec<ToolCatalogueEntry>) {
        for mut entry in entries {
            let key = tool_catalogue_key(&entry);
            if entry.profile_scope.is_none() && entry.profile_enabled.is_none() {
                if let Some(scope) = self
                    .tool_catalogue
                    .get(&key)
                    .and_then(effective_profile_scope)
                {
                    entry.profile_scope = Some(scope);
                    entry.profile_enabled = Some(scope != ToolScope::None);
                }
            }
            self.tool_catalogue.insert(key, entry);
        }
    }

    pub(super) fn replace_tool_catalogue(&mut self, entries: Vec<ToolCatalogueEntry>) {
        let previous = &self.tool_catalogue;
        self.tool_catalogue = entries
            .into_iter()
            .map(|mut entry| {
                let key = tool_catalogue_key(&entry);
                if entry.profile_scope.is_none() && entry.profile_enabled.is_none() {
                    if let Some(scope) = previous.get(&key).and_then(effective_profile_scope) {
                        entry.profile_scope = Some(scope);
                        entry.profile_enabled = Some(scope != ToolScope::None);
                    }
                }
                (key, entry)
            })
            .collect();
        self.open_pending_tool_policy_modal_after_catalogue_update();
    }

    pub(super) fn merge_tool_policy_results(
        &mut self,
        results: Vec<crate::protocol::client::ToolPolicyResult>,
    ) {
        let entries = results
            .into_iter()
            .filter_map(|result| result.after)
            .collect();
        self.merge_tool_catalogue_entries(entries);
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
        let payload = crate::protocol::presentation_payloads::parse_turn_end(&message);
        let refs = payload.message_refs;
        let content_len = payload.content_length;
        self.maybe_recover_from_refs_with_len(&refs, content_len);
        // `usage` is absent on streaming OpenAI-compatible providers (e.g.
        // Fireworks) because their SSE stream does not carry token counts.
        // Don't gate context-gauge updates on it — `contextTokens` is emitted
        // independently and must still drive the footer.
        let total = payload.usage_total;
        if let (Some(used), Some(window)) = (payload.context_tokens, payload.max_context_tokens) {
            self.master_session
                .footer
                .update_context_usage(used, window);
            self.sessions.context_stats_requested = true;
            // Context came inline, but cost does not ride the turn_end event —
            // refresh session stats (quietly) so the footer cost stays current.
            self.send_session_stats_footer();
        } else if total > 0 && !self.sessions.context_stats_requested {
            self.sessions.context_stats_requested = true;
            self.send_session_stats_footer();
        }
    }

    /// #1060 recovery: common streamed path = zero fetches; miss = get_message
    /// per missing ref with request-id gating. Never blindly append-all (#1075).
    ///
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
        let message = crate::components::ansi::sanitize_control(&message);
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
        let Some(agent_id) = crate::protocol::presentation_payloads::string_field(args, "agent_id")
        else {
            return;
        };
        let sanitized = crate::components::ansi::sanitize_control(&agent_id);
        // If the kernel already confirmed this id (a non-optimistic entry), do
        // not clobber it with a fresh unconfirmed "starting" guess. A re-played
        // or duplicate spawn ToolStart (event replay on reconnect) would
        // otherwise reset started_at/status and re-mark it optimistic, partly
        // re-opening the #831 drop path for the grace window (review).
        let tracked = &self.subagents.tracked;
        if tracked.get(&sanitized).is_some_and(|e| !e.optimistic) {
            return;
        }
        let mut tracked = TrackedSubagent::new(crate::protocol::client::SubagentInfoEvent {
            agent_uuid: None,
            display_name: None,
            agent_id: sanitized.clone(),
            status: "starting".to_string(),
            last_tool: None,
            last_error: None,
            pid: 0,
            socket_path: None,
            parent_id: None,
            workflow: None,
            read_only: spawn_args_are_read_only(args),
            execution_backend: None,
            environment: None,
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
        let result_text = crate::protocol::client::extract_result_text(&result);
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
        let sanitized = crate::components::ansi::sanitize_control(agent_id);
        // Prefer durable UUID identity when the spawn result includes it so the
        // optimistic row reconciles with UUID-keyed snapshots (#1378).
        let uuid = result_text
            .find("uuid=")
            .map(|i| &result_text[i + 5..])
            .and_then(|rest| {
                let end = rest
                    .find(|c: char| c == ',' || c == ')' || c.is_whitespace())
                    .unwrap_or(rest.len());
                let candidate = rest[..end].trim();
                (!candidate.is_empty()).then(|| candidate.to_string())
            })
            .map(|u| crate::components::ansi::sanitize_control(&u));
        if let Some(uuid_key) = uuid {
            if let Some(mut entry) = self.subagents.tracked.remove(&sanitized) {
                entry.info.status = "running".to_string();
                entry.info.agent_uuid = Some(uuid_key.clone());
                if entry.info.display_name.is_none() {
                    entry.info.display_name = Some(sanitized.clone());
                }
                self.subagents.tracked.insert(uuid_key.clone(), entry);
                // Rekey sessions/feeds/session_order/active with tracked (#1378).
                self.rekey_agent_collections(&sanitized, &uuid_key);
                return;
            }
            // Already keyed by UUID (or no optimistic row) — just flip status.
            if let Some(entry) = self.subagents.tracked.get_mut(&uuid_key) {
                entry.info.status = "running".to_string();
                entry.info.agent_uuid = Some(uuid_key);
                if entry.info.display_name.is_none() {
                    entry.info.display_name = Some(sanitized);
                }
            }
            return;
        }
        if let Some(entry) = self.subagents.tracked.get_mut(&sanitized) {
            entry.info.status = "running".to_string();
            if entry.info.display_name.is_none() {
                entry.info.display_name = Some(sanitized);
            }
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
        // Forwarded background subagent workflow_state events must update the
        // left-panel data model immediately, but must not clobber the connected
        // session's own workflow bar. Compare to the connected id so a
        // *named*/resumed agent still updates its main bar.
        if let Some(id) = agent_id.as_deref() {
            if self.connected_agent_id.as_deref() != Some(id) {
                let bar = build_workflow_state(
                    &steps,
                    &progress,
                    &active_issue,
                    &mode,
                    &active_template,
                    &available_templates,
                );
                if bar.has_no_progress()
                    && !bar.signals_end_or_reset()
                    && self.subagent_workflow_visible(id)
                {
                    return;
                }
                self.record_subagent_workflow(id, &bar);
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

fn tool_catalogue_key(entry: &ToolCatalogueEntry) -> String {
    if entry.stable_id.is_empty() {
        entry.name.clone()
    } else {
        entry.stable_id.clone()
    }
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
    crate::components::ansi::sanitize_control(
        crate::protocol::presentation_payloads::string_field(args, key)
            .as_deref()
            .unwrap_or(fallback),
    )
}

fn spawn_args_are_read_only(args: &serde_json::Value) -> bool {
    crate::protocol::presentation_payloads::spawn_is_read_only(args)
}

pub(super) fn suppress_tool_box(tool_name: &str, _args: &serde_json::Value) -> bool {
    crate::agents::suppress_tool_box(tool_name)
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
#[path = "app_events_spawn_tests.rs"]
mod spawn_tests;
#[cfg(test)]
#[path = "app_events_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "app_events_1172_tests.rs"]
mod tests_1172;
