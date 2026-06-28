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
        self.sessions.contains_key(id) || self.subagent_local.contains_key(id)
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
            if let Some(session) = self.sessions.get_mut(target) {
                session.workflow_bar = super::app_events::build_workflow_state(
                    steps,
                    progress,
                    active_issue,
                    mode,
                    active_template,
                    available_templates,
                );
            }
            return;
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
        let Some(session) = self.sessions.get_mut(agent_id) else {
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
        let chat = &mut session.chat;
        match ev {
            Event::Token { token } => chat.append_token(&token),
            Event::AgentEnd { .. } | Event::TurnEnd { .. } => chat.finalize_assistant(),
            Event::ToolExecutionStart {
                tool_call_id,
                tool_name,
                args,
            } => {
                let args_str = if args.is_object() || args.is_array() {
                    serde_json::to_string(&args).unwrap_or_default()
                } else {
                    args.to_string()
                };
                if !super::app_events::suppress_tool_box(&tool_name, &args) {
                    chat.start_tool(tool_call_id, tool_name, args_str);
                }
            }
            Event::ToolExecutionEnd {
                tool_call_id,
                result,
                is_error,
                ..
            } => {
                let text = crate::infrastructure::client::extract_result_text(&result);
                chat.complete_tool(&tool_call_id, &text, is_error, None);
            }
            Event::Response {
                command,
                data: Some(data),
                ..
            } if command == "get_messages" => {
                Self::reconcile_backfill_history(session, &data);
                return;
            }
            _ => {}
        }
        if flush_notes {
            Self::flush_deferred_notes(&mut session.chat, &mut session.deferred_subagent_notes);
        }
    }

    /// Reconcile a connect-on-select `get_messages` backfill into a session
    /// (#828). The prior conversation is PREPENDED above whatever live content
    /// already streamed (never a wholesale replace that drops live tokens), and
    /// is applied at most once so a re-delivered backfill cannot duplicate it.
    fn reconcile_backfill_history(session: &mut SessionView, data: &serde_json::Value) {
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
        session.chat.prepend_history(history);
        session.history_backfilled = true;
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
    pub(super) fn flush_deferred_notes(
        chat: &mut Chat,
        deferred: &mut std::collections::VecDeque<String>,
    ) {
        for note in deferred.drain(..) {
            chat.add_entry(ChatEntry::Status {
                text: format!("◆ {note}"),
            });
        }
    }
}
