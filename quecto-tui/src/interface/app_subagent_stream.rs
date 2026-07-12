//! Routing a sub-agent's live UDS stream into its `SessionView` (#800/
//! #828). Split out of `app_subagent_panel.rs` to keep that file within the
//! source line cap; the master and per-session note defer/flush policy and the
//! connect-on-select history backfill reconcile live here in ONE place.

use super::*;

/// Defensive cap on a deferred sub-agent-note buffer (master or per-session,
/// #828): the OLDEST note is evicted past the cap so a chatty grandchild during
/// a long parent turn cannot grow it without bound; newest notes always survive.
pub(super) const DEFERRED_NOTE_CAP: usize = 256;

impl App {
    /// Is `id` an agent we still render — either live-tracked or with a retained
    /// (post-exit) session? The drop-stale invariant (#800): a stale frame or a
    /// forwarded id we don't track must never resurrect/create a session. Hoisted
    /// so the predicate lives in ONE place instead of being copied per guard site.
    fn is_tracked_agent(&self, id: &str) -> bool {
        self.subagents.sessions.contains_key(id) || self.subagents.tracked.contains_key(id)
    }

    /// Route one event from a sub-agent's direct connection into that agent's
    /// `SessionView`, mirroring the master render path so the body is visibly
    /// equivalent to how the master renders (#800).
    pub(super) fn route_subagent_event(&mut self, agent_id: &str, ev: Event) {
        // Per-session workflow bar so a selected sub-agent renders its OWN
        // workflow/phase bar (#802). The kernel re-broadcasts a descendant's
        // `workflow_state` onto an ancestor's stream, re-stamped with the
        // descendant's own inner `agent_id` (#840 / `canonical_workflow_forward`).
        // Such a forwarded event is tagged here with the CONNECTION's id, so
        // route by the event's INNER `agent_id` when present: a grandchild G's
        // workflow must land on G's session, never overwrite the connected
        // child's bar. The connected agent's own events carry no inner id, so
        // they fall back to the connection id.
        if let Event::WorkflowState {
            agent_id: inner_id,
            steps,
            progress,
            active_issue,
            mode,
            active_template,
            available_templates,
        } = &ev
        {
            let target = inner_id.as_deref().unwrap_or(agent_id);
            // Trust assumption (security #856 review): `inner_id` is set by the
            // kernel's `canonical_workflow_forward`, which only re-stamps a true
            // descendant's id onto an ancestor's stream; we do NOT re-verify the
            // ancestry here. The drop-stale guard below still confines the write
            // to an already-tracked/retained session, so a misbehaving id can at
            // worst overwrite another visible agent's workflow bar (display-only,
            // no privilege/data crossover) and can never create a session.
            if !self.is_tracked_agent(target) {
                return;
            }
            self.ensure_session(target);
            let bar = super::app_events::build_workflow_state(
                steps,
                progress,
                active_issue,
                mode,
                active_template,
                available_templates,
            );
            // Stickiness (#901): a transient/empty live `workflow_state` event
            // (e.g. `progress {done:0,total:0}` / no issue, emitted around a
            // transition or nudge) must NOT blank an already-visible workflow.
            // Keep the existing bar/panel unless the new one carries real
            // content or explicitly signals a genuine workflow end/reset. Real
            // progress events (and the show-on-select `0/N` snapshot) still
            // update normally, preserving #869.
            //
            // #915: the guard keys on `has_no_progress()` (no steps + `0/0` + no
            // templates) rather than `is_empty()`, so a TRANSIENT `0/0`-with-issue
            // event — which carries an `activeIssue` but no real progress — also
            // cannot regress an advanced bar (e.g. `2/18`) back to `starting…`.
            // Only genuine progress, or an explicit end/reset, updates the bar.
            if bar.has_no_progress()
                && !bar.signals_end_or_reset()
                && self.subagent_workflow_visible(target)
            {
                return;
            }
            // Mirror onto the panel entry too so the LEFT side panel renders the
            // child's own live progress immediately (#869b).
            self.record_subagent_workflow(target, &bar);
            if let Some(session) = self.subagents.sessions.get_mut(target) {
                session.workflow_bar = bar;
            }
            return;
        }
        // A connect-time / polled `get_state` snapshot for THIS connection carries
        // the child's workflow data (#842). Mirror the master path
        // (`app_response.rs`) so a child viewed MID-workflow renders its bar at
        // once, instead of waiting for the next live `workflow_state` transition
        // (#869a). A `get_state` response is NOT forwarded across streams, so it
        // is keyed by the connection id; the guard still confines the write to an
        // already-tracked/retained session.
        if let Event::Response {
            command,
            success: false,
            error,
            ..
        } = &ev
        {
            if command == "set_effort" {
                if self.subagents.active_agent_id.as_deref() == Some(agent_id) {
                    let detail = error.as_deref().unwrap_or("unknown error");
                    self.notify(
                        &format!("Effort switch failed: {detail}"),
                        NotifyLevel::Error,
                    );
                }
                return;
            }
            if command == "set_model" {
                if self.subagents.active_agent_id.as_deref() == Some(agent_id) {
                    let detail = error.as_deref().unwrap_or("unknown error");
                    self.notify(
                        &format!("Model switch failed: {detail}"),
                        NotifyLevel::Error,
                    );
                }
                return;
            }
        }
        // Production set_model acks with `data: None` (uds.rs AgentEvent::ok
        // with no payload). Match success independently of data so toast +
        // child get_state resync always run (#1085 review).
        if let Event::Response {
            command,
            success: true,
            data,
            ..
        } = &ev
        {
            if command == "set_model" {
                if let Some(model) = data
                    .as_ref()
                    .and_then(|d| d.get("model"))
                    .and_then(|v| v.as_str())
                    .map(crate::interface::ansi::sanitize_control)
                {
                    if let Some(session) = self.subagents.sessions.get_mut(agent_id) {
                        session.footer.set_model(&model);
                    }
                    if self.subagents.active_agent_id.as_deref() == Some(agent_id) {
                        self.current_model = Some(model);
                    }
                }
                if self.subagents.active_agent_id.as_deref() == Some(agent_id) {
                    self.notify("Model switched", NotifyLevel::Success);
                    // Re-sync on the child's own connection so effort vocabulary
                    // tracks the new model (agent resets effort to low on switch).
                    let _ = self.send_to_active_subagent(Command::GetState {
                        id: Some("resync".into()),
                    });
                }
                return;
            }
        }
        if let Event::Response {
            command,
            data: Some(data),
            ..
        } = &ev
        {
            if command == "get_state" {
                if !self.is_tracked_agent(agent_id) {
                    return;
                }
                self.ensure_session(agent_id);
                if let Some(wf) = data.get("workflow") {
                    let bar = workflow_bar::parse_workflow_event(wf);
                    self.record_subagent_workflow(agent_id, &bar);
                    if let Some(session) = self.subagents.sessions.get_mut(agent_id) {
                        session.workflow_bar = bar;
                    }
                }
                // Preserve the existing per-session footer mapping (model +
                // context window) that the generic path applied for get_state.
                if let Some(session) = self.subagents.sessions.get_mut(agent_id) {
                    Self::update_session_footer(session, &ev);
                }
                if self.subagents.active_agent_id.as_deref() == Some(agent_id) {
                    if let Some(model) = data
                        .get("model")
                        .and_then(|v| v.as_str())
                        .map(crate::interface::ansi::sanitize_control)
                    {
                        self.current_model = Some(model);
                    }
                    self.current_effort = data
                        .get("effort")
                        .and_then(|v| v.as_str())
                        .map(crate::interface::ansi::sanitize_control);
                    if let Some(levels) = data.get("effortLevels").and_then(|v| v.as_array()) {
                        let levels: Vec<String> = levels
                            .iter()
                            .filter_map(|l| l.as_str())
                            .map(crate::interface::ansi::sanitize_control)
                            .collect();
                        if !levels.is_empty() {
                            self.effort_levels = levels;
                        }
                    }
                }
                return;
            }
            if command == "set_effort" {
                if let Some(level) = data
                    .get("effort")
                    .and_then(|v| v.as_str())
                    .map(crate::interface::ansi::sanitize_control)
                {
                    if let Some(session) = self.subagents.sessions.get_mut(agent_id) {
                        session.footer.set_effort(Some(level.clone()));
                    }
                    if self.subagents.active_agent_id.as_deref() == Some(agent_id) {
                        self.current_effort = Some(level.clone());
                        self.notify(&format!("Effort set to {level}"), NotifyLevel::Success);
                    }
                }
                return;
            }
        }
        // Tearing down a connection mid-stream can leave already-queued
        // `(old_id, ev)` items in `subagent_event_rx`. Drop events for agents
        // that are neither still tracked nor have a retained session, so a
        // stale frame cannot resurrect a session `evict_retained_sessions`
        // just dropped (#800 review).
        if !self.is_tracked_agent(agent_id) {
            return;
        }
        self.ensure_session(agent_id);
        let Some(session) = self.subagents.sessions.get_mut(agent_id) else {
            return;
        };
        match &ev {
            Event::AgentStart | Event::TurnStart => {
                session.running = true;
                session.observed_run_state = true;
                session.footer.set_streaming(true);
            }
            Event::AgentEnd { .. } | Event::TurnEnd { .. } => {
                session.running = false;
                session.observed_run_state = true;
                session.footer.set_streaming(false);
            }
            _ => {}
        }
        // Per-session FOOTER: feed the child's OWN context-window / cost / model
        // gauges from its forwarded events so a selected sub-agent shows ITS
        // usage, not the master's (#805).
        Self::update_session_footer(session, &ev);
        // A completion note for THIS child's own sub-agent (a grandchild): render
        // it as a passive one-line status in this session's chat, deferred while
        // the child streams so it never splits the child's response (#816).
        if let Event::SubagentNotification { message, .. } = &ev {
            let message = crate::interface::ansi::sanitize_control(message);
            Self::push_or_defer_note(
                &mut session.chat,
                &mut session.deferred_subagent_notes,
                session.running,
                message,
            );
            return;
        }
        // Flush deferred grandchild notes once this child goes idle (after the
        // streamed response is finalized below).
        let flush_notes = matches!(ev, Event::AgentEnd { .. } | Event::TurnEnd { .. });
        let recovery_refs = Self::subagent_end_of_turn_refs(&ev);
        let early_return = matches!(
            &ev,
            Event::Response { command, .. }
                if command == "get_messages" || command == "get_message"
        );
        Self::apply_subagent_chat_event(session, &ev);
        if early_return {
            return;
        }
        if flush_notes {
            Self::flush_deferred_notes(&mut session.chat, &mut session.deferred_subagent_notes);
        }
        if let Some((refs, content_len)) = recovery_refs {
            self.maybe_recover_subagent_refs(agent_id, &refs, content_len);
        }
    }

    /// Extract non-empty end-of-turn message refs (+ optional contentLength).
    fn subagent_end_of_turn_refs(ev: &Event) -> Option<(Vec<String>, Option<u64>)> {
        match ev {
            Event::AgentEnd { message_refs, .. } if !message_refs.is_empty() => {
                Some((message_refs.clone(), None))
            }
            Event::TurnEnd { message } => {
                let refs = message
                    .get("messageRefs")
                    .and_then(|r| r.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .filter(|s| !s.is_empty())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let len = message.get("contentLength").and_then(|v| v.as_u64());
                if refs.is_empty() {
                    None
                } else {
                    Some((refs, len))
                }
            }
            _ => None,
        }
    }

    /// Apply a single non-workflow child stream event to its chat.
    fn apply_subagent_chat_event(session: &mut SessionView, ev: &Event) {
        let chat = &mut session.chat;
        match ev {
            Event::Token { token } => chat.append_token(token),
            Event::AgentEnd { .. } | Event::TurnEnd { .. } => chat.finalize_assistant(),
            Event::ToolExecutionStart {
                tool_call_id,
                tool_name,
                args,
            } => {
                let args_str = if args.is_object() || args.is_array() {
                    serde_json::to_string(args).unwrap_or_default()
                } else {
                    args.to_string()
                };
                if super::app_events::suppress_tool_box(tool_name, args) {
                    chat.finalize_assistant();
                } else {
                    chat.start_tool(tool_call_id.clone(), tool_name.clone(), args_str);
                }
            }
            Event::ToolExecutionEnd {
                tool_call_id,
                result,
                is_error,
                ..
            } => {
                let text = crate::infrastructure::client::extract_result_text(result);
                chat.complete_tool(tool_call_id, &text, *is_error, None);
            }
            Event::Response {
                command,
                data: Some(data),
                ..
            } if command == "get_messages" => {
                Self::reconcile_backfill_history(session, data);
            }
            Event::Response {
                id,
                command,
                success,
                data,
                ..
            } if command == "get_message" => {
                Self::apply_subagent_get_message_recovery(
                    session,
                    id.as_deref(),
                    *success,
                    data.as_ref(),
                );
            }
            _ => {}
        }
    }

    /// #1060: fetch missing child messages by ref on the child UDS connection.
    fn maybe_recover_subagent_refs(
        &mut self,
        agent_id: &str,
        refs: &[String],
        expected_content_len: Option<u64>,
    ) {
        if refs.is_empty() {
            return;
        }
        let session = match self.subagents.sessions.get(agent_id) {
            Some(s) => s,
            None => return,
        };
        let assistant_text = session
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
            .unwrap_or_default();
        let tools_this_turn = session.chat.tool_entry_count(); // child session is one turn's view often
        // Reuse master heuristic with child chat state.
        if !self.needs_message_recovery_for(
            refs,
            &assistant_text,
            tools_this_turn,
            expected_content_len,
        ) {
            return;
        }
        for message_id in refs {
            if self
                .pending_message_recovery
                .values()
                .any(|pending| pending == message_id)
            {
                continue;
            }
            let req_id = format!("msg-recovery-{}", {
                use std::time::{SystemTime, UNIX_EPOCH};
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            });
            self.pending_message_recovery
                .insert(req_id.clone(), message_id.clone());
            let cmd = Command::GetMessage {
                id: Some(req_id),
                message_id: message_id.clone(),
                agent_id: None, // sent on the child's own socket
            };
            let _ = self.send_to_active_subagent(cmd);
        }
    }

    fn apply_subagent_get_message_recovery(
        session: &mut SessionView,
        id: Option<&str>,
        success: bool,
        data: Option<&serde_json::Value>,
    ) {
        // Request-id gating is enforced by the caller having filtered pending map
        // on the master path; child path accepts successful get_message payloads
        // that carry content (child connection is single-client).
        let _ = id;
        if !success {
            return;
        }
        let Some(data) = data else {
            return;
        };
        let role = data.get("role").and_then(|r| r.as_str()).unwrap_or("");
        let content = data
            .get("content")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();
        match role {
            "assistant" if !content.is_empty() => {
                session.chat.reconcile_assistant_text(&content);
            }
            "tool" => {
                let tool_call_id = data
                    .get("toolCallId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let tool_name = data
                    .get("toolName")
                    .and_then(|v| v.as_str())
                    .unwrap_or("tool")
                    .to_string();
                if tool_call_id.is_empty() {
                    return;
                }
                if !session.chat.has_tool_call(&tool_call_id) {
                    session
                        .chat
                        .start_tool(tool_call_id.clone(), tool_name, String::new());
                }
                let is_error = data
                    .get("isError")
                    .or_else(|| data.get("is_error"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                session
                    .chat
                    .complete_tool(&tool_call_id, &content, is_error, None);
            }
            _ => {}
        }
    }

    /// Seed a just-selected sub-agent's main-pane `workflow_bar` from the
    /// registry snapshot (`subagent_local[id].info.workflow`) the left-panel
    /// cells already render, so the bar appears on select without waiting for a
    /// routed `get_state`/live `workflow_state` (#913). No-op when the snapshot
    /// has no workflow, or when the session already carries a (routed/live) bar
    /// — so a more-detailed live bar is never overwritten by the count-only
    /// snapshot.
    pub(super) fn seed_session_bar_from_snapshot(&mut self, id: &str) {
        let Some(wf) = self.subagents.tracked_workflow(id) else {
            return;
        };
        if wf.steps_total == 0 && wf.steps_completed == 0 {
            return;
        }
        let seeded = workflow_bar::WorkflowBarState::from_subagent_snapshot(
            &wf.mode,
            wf.steps_completed,
            wf.steps_total,
        );
        if let Some(session) = self.subagents.sessions.get_mut(id) {
            if !session.workflow_bar.is_visible() && session.workflow_bar.done == 0 {
                session.workflow_bar = seeded;
            }
        }
    }

    /// Mirror a routed workflow snapshot onto the agent's panel (`subagent_local`)
    /// entry so the LEFT side panel renders the FULL per-step bar — filled markers
    /// up to `done` and empty markers up to `total` (e.g. 3/20 = 3 filled + 17
    /// empty) — from the child's OWN live `workflow_state` / `get_state`, not only
    /// the periodic `subagent_state_changed` push (#869b). Preserves BOTH the
    /// completed and total counts so the indicator never collapses to filled-only.
    /// Per-agent keyed, so a descendant's update never overwrites an ancestor row.
    /// Whether `agent_id` currently shows a workflow on EITHER surface — the
    /// LEFT panel cells (`steps_total > 0`) or the main-pane bar
    /// (`workflow_bar.is_visible()`). The stickiness guard (#901) uses this so a
    /// transient/empty event never blanks an indicator that is already visible.
    fn subagent_workflow_visible(&self, agent_id: &str) -> bool {
        let panel_visible = self
            .subagents
            .tracked_workflow(agent_id)
            .is_some_and(|w| w.steps_total > 0);
        let bar_visible = self
            .subagents
            .sessions
            .get(agent_id)
            .is_some_and(|s| s.workflow_bar.is_visible());
        panel_visible || bar_visible
    }

    fn record_subagent_workflow(&mut self, agent_id: &str, bar: &workflow_bar::WorkflowBarState) {
        if let Some(tracked) = self.subagents.tracked.get_mut(agent_id) {
            let mode = tracked
                .info
                .workflow
                .as_ref()
                .map(|w| w.mode.clone())
                .unwrap_or_else(|| "active".to_string());
            tracked.info.workflow = Some(crate::infrastructure::client::SubagentWorkflow {
                mode,
                steps_completed: bar.done,
                steps_total: bar.total,
            });
        }
    }

    /// Reconcile a connect-on-select / attach-time `get_messages` backfill into
    /// a session (#828 master attach #1050). The prior conversation is
    /// PREPENDED above whatever live content already streamed (never a
    /// wholesale replace that drops live tokens), and a complete (untrimmed)
    /// backfill is applied at most once so a re-delivered payload cannot
    /// duplicate it. Shared by sub-agent connect-on-select and master
    /// `--socket` attach.
    ///
    /// Busy-connect snapshots may set `trimmed: true` when the producer drops
    /// oldest messages to stay under the frame budget (`uds_snapshots`). Those
    /// partial tails are applied for immediate display but do **not** latch
    /// `history_backfilled`; a later complete attach-backfill / get_messages
    /// replaces the partial prefix so omitted older history is not permanently
    /// suppressed (#1050 review).
    pub(super) fn reconcile_backfill_history(session: &mut SessionView, data: &serde_json::Value) {
        use crate::application::session_payloads::{self, ResumedChatMessage};
        if session.history_backfilled {
            return;
        }
        let Ok(messages) = session_payloads::parse_resumed_messages(data) else {
            return;
        };
        let history: Vec<ChatEntry> = messages
            .into_iter()
            .map(|message| match message {
                // Sub-agent transcript text is untrusted; strip terminal control
                // sequences before rendering so a child cannot inject ANSI/OSC/
                // bidi escapes into the operator's terminal (#828 security).
                ResumedChatMessage::User(text) => ChatEntry::User {
                    text: crate::interface::ansi::sanitize_control_keep_newlines(&text),
                },
                ResumedChatMessage::Assistant(text) => ChatEntry::Assistant {
                    text: crate::interface::ansi::sanitize_control_keep_newlines(&text),
                    streaming: false,
                },
            })
            .collect();
        // Only mark the backfill applied once it actually carried content: an
        // empty/filtered payload must not latch the guard and permanently
        // suppress a later populated backfill (reconnect / response racing ahead
        // of persistence) (#828 review).
        if history.is_empty() {
            return;
        }
        let history_len = history.len();
        let trimmed = data
            .get("trimmed")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        // A prior trimmed snapshot already contributed a prefix: replace it so
        // the fuller payload does not stack another copy of the same tail.
        if let Some(partial_len) = session.partial_backfill_len {
            session.chat.replace_history_prefix(partial_len, history);
        } else {
            session.chat.prepend_history(history);
        }
        if trimmed {
            session.partial_backfill_len = Some(history_len);
        } else {
            session.partial_backfill_len = None;
            session.history_backfilled = true;
        }
    }

    /// Render a passive sub-agent completion note, or DEFER it while the owning
    /// agent is mid-turn so it never splits an in-flight streaming response
    /// (#816). Shared by the master and per-session paths (#828) — one place for
    /// the defer/flush policy. The deferred buffer is capped (`DEFERRED_NOTE_CAP`)
    /// by evicting the oldest note, so a chatty grandchild cannot grow it without
    /// bound.
    pub(super) fn push_or_defer_note(
        chat: &mut Chat,
        deferred: &mut std::collections::VecDeque<String>,
        running: bool,
        message: String,
    ) {
        if running {
            if deferred.len() >= DEFERRED_NOTE_CAP {
                deferred.pop_front();
            }
            deferred.push_back(message);
        } else {
            chat.add_entry(ChatEntry::Status {
                text: format!("◆ {message}"),
            });
        }
    }

    /// Flush all deferred notes into the chat as passive status lines, in order
    /// (#816/#828). Counterpart to `push_or_defer_note`.
    ///
    /// When MORE THAN ONE normal turn-end note drains together at the idle
    /// boundary, collapse them into ONE coalesced `◆` summary line listing the
    /// names (capped, with a `(+M more)` tail) — the display analogue of the
    /// context-side coalescing #894 (#900). A single completion keeps its own
    /// verbatim line; errored/exited notes never fold and pass through as their
    /// own `◆` lines, preserving their failure detail. The coalesced summary
    /// takes the position of the FIRST completion so ordering is stable.
    pub(super) fn flush_deferred_notes(
        chat: &mut Chat,
        deferred: &mut std::collections::VecDeque<String>,
    ) {
        let drained: Vec<String> = deferred.drain(..).collect();
        let names: Vec<&str> = drained
            .iter()
            .filter_map(|m| Self::completion_note_name(m))
            .collect();
        // Fewer than two completions: nothing to coalesce — emit verbatim.
        if names.len() < 2 {
            for note in &drained {
                chat.add_entry(ChatEntry::Status {
                    text: format!("◆ {note}"),
                });
            }
            return;
        }
        let summary = Self::coalesced_completion_summary(&names);
        let mut emitted_summary = false;
        for note in &drained {
            if Self::completion_note_name(note).is_some() {
                // Replace the run of completions with one summary at the first.
                if !emitted_summary {
                    chat.add_entry(ChatEntry::Status {
                        text: format!("◆ {summary}"),
                    });
                    emitted_summary = true;
                }
            } else {
                chat.add_entry(ChatEntry::Status {
                    text: format!("◆ {note}"),
                });
            }
        }
    }

    /// Detect a normal turn-end note and extract the sub-agent name (#900).
    /// Mirrors the kernel wording `Sub-agent '<name>' ended a turn (status: idle).` emitted by
    /// `SubagentNotification::Completed::to_message` (subagent_registry). Errored
    /// (`Agent '<name>' failed: …`) and exited notes do NOT match, so they fall
    /// through as their own verbatim `◆` lines.
    fn completion_note_name(message: &str) -> Option<&str> {
        let rest = message.strip_prefix("Sub-agent '")?;
        let end = rest.find("' ended a turn (status: idle).")?;
        Some(&rest[..end])
    }

    /// Build the body of a coalesced completion summary:
    /// `"N sub-agents ended a turn: a, b, c (+M more)"`, capping the listed names at
    /// [`COALESCE_NAME_CAP`] (#900), mirroring #894's context-side cap.
    fn coalesced_completion_summary(names: &[&str]) -> String {
        let total = names.len();
        let shown = total.min(COALESCE_NAME_CAP);
        let mut list = names[..shown].join(", ");
        if total > shown {
            list.push_str(&format!(" (+{} more)", total - shown));
        }
        format!(
            "{total} sub-agents ended a turn: {list}. Inspect agent_cmd get_messages before treating their work as complete."
        )
    }
}

/// Maximum number of sub-agent names listed verbatim in a coalesced completion
/// summary line before the remainder collapses to a `(+M more)` tail (#900).
const COALESCE_NAME_CAP: usize = 10;
