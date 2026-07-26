use super::*;
/// Defensive cap on deferred sub-agent notes; oldest notes are evicted.
pub(super) const DEFERRED_NOTE_CAP: usize = 256;

impl App {
    /// Is `id` still rendered (live-tracked or retained post-exit)? The drop-stale
    /// invariant (#800): stale/untracked frames must never resurrect sessions.
    pub(super) fn is_retained_or_tracked_agent(&self, id: &str) -> bool {
        self.subagents.sessions.contains_key(id) || self.subagents.tracked.contains_key(id)
    }

    /// Route one event from a sub-agent's direct connection into that agent's
    /// `SessionView`, mirroring the master render path so the body is visibly
    /// equivalent to how the master renders (#800).
    pub(super) fn route_subagent_event(&mut self, agent_id: &str, ev: Event) {
        // Route forwarded descendant workflow by its inner id; otherwise use the connection id.
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
            if !self.is_retained_or_tracked_agent(target) {
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
        // A get_state snapshot for THIS connection carries the child's workflow
        // data (#842/#869a); it is keyed by the connection id, not forwarded.
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
                        self.inference.current_model = Some(model);
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
        if let Event::LedgerAdvanced { epoch, rev } = &ev {
            self.note_ledger_advanced(agent_id, *epoch, *rev);
            return;
        }
        if let Event::Response {
            command,
            data: Some(data),
            ..
        } = &ev
        {
            if command == "sync" {
                self.route_sync_response(agent_id, data);
                return;
            }
            if command == "get_state" {
                if !self.is_retained_or_tracked_agent(agent_id) {
                    return;
                }
                self.ensure_session(agent_id);
                self.note_sync_capability(agent_id, data);
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
                        self.inference.current_model = Some(model);
                    }
                    self.inference.current_effort = data
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
                            self.inference.effort_levels = levels;
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
                        self.inference.current_effort = Some(level.clone());
                        self.notify(&format!("Effort set to {level}"), NotifyLevel::Success);
                    }
                }
                return;
            }
        }
        // Tearing down a connection mid-stream can leave queued events. Drop events
        // for agents no longer tracked or retained so stale frames cannot resurrect
        // sessions just dropped by `evict_retained_sessions` (#800 review).
        if !self.is_retained_or_tracked_agent(agent_id) {
            return;
        }
        if let Event::SubagentStateChanged { subagents } = ev {
            // Retained history does not grant authority to publish topology.
            if self.subagents.tracked.contains_key(agent_id) {
                self.update_subagent_bar_from_source(Some(agent_id), subagents);
            }
            return;
        }
        self.ensure_session(agent_id);
        let synced_authoritative = self.is_synced_authoritative_feed(agent_id);
        let focused_live = self.subagents.active_agent_id.as_deref() == Some(agent_id);
        let Some(session) = self.subagents.sessions.get_mut(agent_id) else {
            return;
        };
        let was_running = session.running;
        match &ev {
            Event::AgentStart | Event::TurnStart => {
                if !session.running {
                    session.active_turn_start = session.chat.entry_count();
                    // New turn: reset the per-turn tool count that drives
                    // end-of-turn ref-cardinality recovery (#1060 review, F2).
                    session.tools_this_turn = 0;
                    session.open_tool_calls = 0;
                }
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
            Event::Response { command, .. } if command == "get_messages"
        );
        if Self::apply_subagent_chat_event_or_skip(
            session,
            &ev,
            synced_authoritative,
            focused_live,
            was_running,
            early_return,
        ) {
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

    fn apply_subagent_chat_event_or_skip(
        session: &mut SessionView,
        ev: &Event,
        synced: bool,
        focused_live: bool,
        was_running: bool,
        early: bool,
    ) -> bool {
        if synced && !focused_live {
            if was_running && !session.running {
                session.chat.finalize_assistant();
            }
            return early;
        }
        if synced && focused_live && !was_running {
            return early;
        }
        Self::apply_subagent_chat_event(session, ev);
        early
    }

    /// Apply a single non-workflow child stream event to its chat.
    fn apply_subagent_chat_event(session: &mut SessionView, ev: &Event) {
        // Maintain the per-turn tool count (ref cardinality, F2) and the
        // open-tool-call count (dropped tool-end recovery, review 3) before
        // borrowing `chat`, mirroring the master path (#1060).
        if matches!(ev, Event::ToolExecutionStart { .. }) {
            session.tools_this_turn = session.tools_this_turn.saturating_add(1);
            session.open_tool_calls = session.open_tool_calls.saturating_add(1);
        } else if matches!(ev, Event::ToolExecutionEnd { .. }) {
            session.open_tool_calls = session.open_tool_calls.saturating_sub(1);
        }
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
            Event::Response { command, id, .. }
                if command == "get_messages" && session.is_pending_history_page(id.as_deref()) =>
            {
                // Failed / ignored child page fetch: clear the in-flight request so
                // a future ledger-sync compatible pagination path can retry cleanly.
                session.clear_pending_history_page();
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
        let Some(session) = self.subagents.sessions.get(agent_id) else {
            return;
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
        // Per-turn tool count, not session-lifetime `tool_entry_count()`, which
        // over-counts on later turns and forces false-positive recovery (F2).
        let tools = session.tools_this_turn;
        // A dropped child tool-end leaves a result unresolved; the policy forces
        // recovery even when cardinality looks complete (#1060 review 3).
        if !(crate::domain::turn_recovery::TurnOutcome {
            refs,
            assistant_text: &assistant_text,
            tools_this_turn: tools,
            open_tool_calls: session.open_tool_calls,
            expected_content_len,
        })
        .needs_recovery()
        {
            return;
        }
        // If any ref is already being recovered, the batch that owns it will
        // fill; creating a second batch here would issue zero fresh requests and
        // linger unfillable (F4 — mirrors the master guard).
        if refs.iter().any(|message_id| {
            self.pending_message_recovery
                .values()
                .any(|pending| pending.message_id == *message_id)
        }) {
            return;
        }
        let batch_id = format!(
            "child-recovery-{agent_id}-{}",
            super::app_events::uuid_like()
        );
        let target_end = session.chat.entry_count();
        self.message_recovery_batches.insert(
            batch_id.clone(),
            MessageRecoveryBatch::new(
                refs.to_vec(),
                session.active_turn_start.min(target_end),
                target_end,
                Some(agent_id.to_string()),
            ),
        );
        for message_id in refs {
            let req_id = format!("msg-recovery-{}", super::app_events::uuid_like());
            self.pending_message_recovery.insert(
                req_id.clone(),
                PendingMessageRecovery {
                    message_id: message_id.clone(),
                    batch_id: batch_id.clone(),
                    agent_id: Some(agent_id.to_string()),
                    content: String::new(),
                    offset: 0,
                },
            );
            // Route via the MASTER connection; it forwards by child id and the
            // response is applied by `handle_get_message_recovery`.
            self.send_command(Command::GetMessage {
                id: Some(req_id),
                message_id: message_id.clone(),
                agent_id: Some(agent_id.to_string()),
                tool_call_id: None,
                offset: Some(0),
                limit: Some(super::app_paged_history::GET_MESSAGE_PAGE_BYTES),
            });
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

    pub(super) fn subagent_workflow_visible(&self, agent_id: &str) -> bool {
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

    pub(super) fn record_subagent_workflow(
        &mut self,
        agent_id: &str,
        bar: &workflow_bar::WorkflowBarState,
    ) {
        if let Some(tracked) = self.subagents.tracked.get_mut(agent_id) {
            tracked.info.workflow = Some(crate::infrastructure::client::SubagentWorkflow {
                mode: bar.mode.clone().unwrap_or_else(|| "active".to_string()),
                steps_completed: bar.done,
                steps_total: bar.total,
            });
        }
    }

    /// Map a backfilled/resumed history message to a chat entry: a ladder-demoted
    /// message carrying a stable id becomes a recallable [`ChatEntry::Stub`];
    /// anything else renders as a plain user/assistant line (#1061). Shared by the
    /// sub-agent/master backfill and the resume path so both recall identically.
    pub(super) fn history_entry(
        text: String,
        id: Option<String>,
        stub: bool,
        is_user: bool,
    ) -> ChatEntry {
        match (stub, id) {
            (true, Some(id)) => ChatEntry::Stub { id, is_user, text },
            _ if is_user => ChatEntry::User { text },
            _ => ChatEntry::Assistant {
                text,
                streaming: false,
            },
        }
    }

    /// Reconcile a master-session `get_messages` snapshot with already-streamed tail entries.
    ///
    /// This is retained for master attach/resume and explicit master history
    /// pagination only; sub-agent transcripts now use ledger `sync` deltas rather
    /// than the deleted parent-forwarded backfill fallback.
    pub(super) fn reconcile_master_backfill_history(
        session: &mut SessionView,
        data: &serde_json::Value,
        extend_prefix: bool,
    ) {
        use crate::application::session_payloads;
        use crate::domain::history_paging::{PageFacts, PrefixPlan};
        if session.history.backfilled {
            return;
        }
        let Ok(messages) = session_payloads::parse_resumed_messages(data) else {
            return;
        };
        let history: Vec<ChatEntry> = Self::resumed_chat_entries(messages);
        let facts = PageFacts {
            before: data
                .get("before")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned),
            has_more_before: data
                .get("hasMoreBefore")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            trimmed: data
                .get("trimmed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            page_len: history.len(),
            extend_prefix,
        };
        // The policy publishes cursors before reporting an empty page, so an
        // empty/filtered payload leaves paging reachable without latching the
        // backfill guard and permanently suppressing a later populated backfill
        // (reconnect / response racing ahead of persistence) (#828 review).
        match session.history.reconcile(&facts) {
            None => {}
            Some(PrefixPlan::Prepend) => session.chat.prepend_history(history),
            Some(PrefixPlan::ReplacePrefix(len)) => {
                session.chat.replace_history_prefix(len, history)
            }
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
