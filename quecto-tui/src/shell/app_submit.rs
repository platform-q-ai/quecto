use super::*;

impl App {
    pub(super) fn handle_submit(&mut self, text: &str) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }

        // Slash commands. Validate the command name against the single source
        // of truth (`builtin_commands`) before dispatching, so the set of valid
        // commands is not re-enumerated here.
        if trimmed.starts_with('/') {
            let name = trimmed
                .split_whitespace()
                .next()
                .unwrap_or(trimmed)
                .trim_start_matches('/');
            if !builtin_commands().iter().any(|c| c.name == name) {
                self.reject_unknown_slash_command(trimmed);
                return;
            }
            match trimmed {
                "/quit" | "/exit" => {
                    self.should_exit = true;
                    return;
                }
                "/clear" => {
                    self.reset_session("Conversation cleared");
                    return;
                }
                "/new" => {
                    for watch in self.reset_workspace() {
                        tokio::spawn(async move {
                            watch.terminate().await;
                        });
                    }
                    return;
                }
                "/tab-new" => {
                    self.open_new_tab_announced();
                    return;
                }
                "/tab-close" => {
                    let tab = self.active_tab;
                    // AC3a / ADR-0023: closing a tab terminates that tab's agent.
                    match self.close_tab(tab, true) {
                        Ok(watch) => {
                            if let Some(w) = watch {
                                tokio::spawn(async move {
                                    w.terminate().await;
                                });
                            }
                            self.notify(
                                &format!("Closed tab {} (agent terminated)", tab.0),
                                crate::components::notification::NotifyLevel::Info,
                            );
                        }
                        Err(msg) => {
                            self.notify(msg, crate::components::notification::NotifyLevel::Warning)
                        }
                    }
                    return;
                }
                "/tab-next" => {
                    let tab = self.switch_tab_next();
                    self.notify(
                        &format!("Active tab {}", tab.0),
                        crate::components::notification::NotifyLevel::Info,
                    );
                    return;
                }
                "/tab-prev" => {
                    let tab = self.switch_tab_prev();
                    self.notify(
                        &format!("Active tab {}", tab.0),
                        crate::components::notification::NotifyLevel::Info,
                    );
                    return;
                }
                "/help" | "/hotkeys" => {
                    self.show_help();
                    return;
                }
                "/session" => {
                    self.send_session_stats();
                    return;
                }
                "/refresh-tui" => {
                    self.terminal.refresh_size();
                    self.render_full();
                    return;
                }
                "/delete-all-subagents" => {
                    self.delete_all_subagents();
                    return;
                }
                "/workflow" => {
                    self.show_workflow_status();
                    return;
                }
                "/resume" => {
                    self.send_list_sessions();
                    return;
                }
                _ if trimmed.starts_with("/resume ") => {
                    let session = trimmed["/resume".len()..].trim();
                    if session.is_empty() {
                        self.send_list_sessions();
                    } else {
                        // Latch when the tab is still connecting / disconnected (AC5).
                        self.queue_or_send_session_resume(session);
                    }
                    return;
                }
                "/thinking" => {
                    self.toggle_thinking_visibility();
                    return;
                }
                _ if trimmed.starts_with("/effort") => {
                    let arg = trimmed["/effort".len()..].trim();
                    self.handle_effort_command(arg);
                    return;
                }
                // Matched before `/model` so an argument cannot fall through and
                // be read as a model name; the argument selects one source.
                _ if trimmed.starts_with("/models-refresh") => {
                    let provider = trimmed["/models-refresh".len()..].trim();
                    let provider = (!provider.is_empty()).then(|| provider.to_string());
                    self.send_models_refresh(provider);
                    return;
                }
                _ if trimmed.starts_with("/model") => {
                    let model_name = trimmed["/model".len()..].trim();
                    if !model_name.is_empty() {
                        self.send_set_model(model_name);
                    } else {
                        // No model name — open the model selector overlay.
                        self.open_model_selector();
                    }
                    return;
                }
                "/workflow-auto" => {
                    self.toggle_workflow_auto_continue();
                    return;
                }
                "/workflow-nudge" => {
                    self.toggle_workflow_completion_nudge();
                    return;
                }
                _ => {
                    self.reject_unknown_slash_command(trimmed);
                    return;
                }
            }
        }

        // Route to the ACTIVE session (#802). A selected sub-agent's prompt
        // targets THAT agent over its own connection and lands in its session,
        // not master's. When the selected child is already running, Enter queues
        // a follow-up behind the current turn; it does not interrupt/steer.
        if let Some(agent_id) = self.ac().roster.active_agent_id.clone() {
            // Attach-on-demand (#1466 round 2): a restored sub-agent may have
            // been focused before its live socket was known, leaving a stale
            // inspection-only feed that cannot carry a Prompt. Re-run the
            // feed attach so a now-usable socket upgrades to a direct feed —
            // the same attach the master-driven roster-refresh path performs.
            self.ensure_synced_subagent_feed(&agent_id);
            // Never silently swallow a message to a non-live sub-agent
            // (#1466 fix pass item 5): a feed channel can exist and accept
            // the enqueue with nothing consuming it, so liveness is judged
            // from the roster's #1461 state, not the channel. "detached" is
            // refused only while the child stays UNREACHABLE — a detached
            // roster row whose registry socket is live just attached above
            // and must deliver (#1466 round 2).
            let status = self
                .ac()
                .roster
                .tracked
                .get(&agent_id)
                .map(|t| t.info.status.clone())
                .unwrap_or_default();
            if crate::agents::roster::subagent_status_is_terminal(&status)
                || (status == "detached" && !self.subagent_feed_is_direct(&agent_id))
            {
                self.note_subagent_undeliverable(&agent_id, &status);
                return;
            }
            let cmd = if self.active_subagent_running() {
                Command::FollowUp {
                    id: None,
                    message: text.to_string(),
                }
            } else {
                Command::Prompt {
                    id: None,
                    message: text.to_string(),
                    streaming_behavior: None,
                }
            };
            // Append to the sub-agent transcript ONLY when the route actually
            // enqueued it (#804 review): a failed route (no live sender / full
            // channel) never delivered the prompt, so a User entry would diverge
            // UI from state.
            if self.send_to_active_subagent(cmd) {
                self.active_chat_mut()
                    .add_entry_follow_tail(ChatEntry::User {
                        text: text.to_string(),
                    });
            } else {
                // Failed route (no live sender / full channel): the user must
                // see the message was not delivered (fix pass item 5).
                self.note_subagent_undeliverable(&agent_id, "unattached");
            }
            return;
        }

        // Pending attach: queue the prompt with connecting UX — not a dead
        // disconnect refusal that tells the user to restart (F7 / AC2).
        if self.ac().pending_attach || self.tab_has_pending_attach(self.active_tab) {
            self.ac_mut()
                .master_session
                .chat
                .add_entry_follow_tail(ChatEntry::User {
                    text: text.to_string(),
                });
            self.ac_mut().queued_prompts.push(text.to_string());
            self.ac_mut()
                .master_session
                .chat
                .add_entry(ChatEntry::Status {
                    text: "Connecting… prompt queued and will send when the agent is ready."
                        .to_string(),
                });
            self.notify(
                "Connecting… prompt queued",
                crate::components::notification::NotifyLevel::Info,
            );
            return;
        }

        // The composed text always lands in the chat (the editor was
        // already emptied by take_submit) — on a dead connection it is the
        // only surviving copy (#1470 r3/r6, single add site).
        self.ac_mut()
            .master_session
            .chat
            .add_entry_follow_tail(ChatEntry::User {
                text: text.to_string(),
            });
        // Refuse when the connection is known dead (#1470): the writer
        // channel can outlive the stream, so an enqueue could "succeed" and
        // the message silently vanish. The persistent refusal Status line
        // keeps the undelivered message diagnosable after the toast expires.
        if !self.ac().agent_connected {
            self.note_disconnected_refusal();
            return;
        }
        self.dispatch_master_user_text(text);
    }

    /// Open a connecting tab with a live persistent agent and announce it —
    /// the ONE new-tab path shared by /tab-new, the clickable " + " button
    /// and the Ctrl+N chord (#1466 round 2; AC1/AC2).
    pub(super) fn open_new_tab_announced(&mut self) {
        let tab = self.open_live_tab(None);
        self.notify(
            &format!("Opened tab {} (connecting…)", tab.0),
            crate::components::notification::NotifyLevel::Info,
        );
    }

    /// Surface an undeliverable sub-agent message (#1466 fix pass item 5):
    /// a persistent Status line plus a toast, so the drop is never silent.
    fn note_subagent_undeliverable(&mut self, agent_id: &str, status: &str) {
        let text = format!("Message not delivered — sub-agent '{agent_id}' is {status}.");
        self.active_chat_mut()
            .add_entry(ChatEntry::Status { text: text.clone() });
        self.notify(&text, crate::components::notification::NotifyLevel::Warning);
    }

    /// Send a master-session user message (Prompt or FollowUp) after connect.
    pub(crate) fn dispatch_master_user_text(&mut self, text: &str) {
        let cmd = if self.ac().agent_state.is_running() {
            Command::FollowUp {
                id: None,
                message: text.to_string(),
            }
        } else {
            Command::Prompt {
                id: None,
                message: text.to_string(),
                streaming_behavior: None,
            }
        };
        self.send_command(cmd);
    }

    /// Deliver prompts queued while the active tab was still connecting (F7).
    pub(crate) fn flush_queued_prompts(&mut self) {
        let queued = std::mem::take(&mut self.ac_mut().queued_prompts);
        for text in queued {
            if !self.ac().agent_connected {
                // Re-queue remainder if the connection dropped mid-flush.
                self.ac_mut().queued_prompts.push(text);
                break;
            }
            // Chat already recorded the User entry at queue time — only send.
            self.dispatch_master_user_text(&text);
        }
    }

    // ── Abort handling (bug fix) ──────────────────────────────────────

    pub(super) fn handle_abort(&mut self) {
        // Abort targets the ACTIVE session (#802): a selected sub-agent's abort is
        // routed over its own connection and finalizes its transcript.
        if self.ac().roster.active_agent_id.is_some() {
            self.send_to_active_subagent(Command::Abort { id: None });
            self.active_chat_mut().finalize_assistant();
            if let Some(id) = self.ac().roster.active_agent_id.clone() {
                if let Some(session) = self.ac_mut().roster.sessions.get_mut(&id) {
                    session.running = false;
                    // Just cancelled: mark run-state observed so the lagging tracked
                    // status can't keep it "running" and re-abort on a 2nd Esc (#834).
                    session.observed_run_state = true;
                }
            }
            return;
        }

        self.send_command(Command::Abort { id: None });

        // Abort the state machine — does NOT set running false; the matched
        // AgentEnd arrives and guards against stale events corrupting state (#502).
        self.ac_mut().agent_state.abort();
        self.ac_mut().master_session.footer.set_streaming(false);

        // Stop spinner / working indicator; `agent_state` stays aborting (#828).
        self.ac_mut().master_session.running = false;
        self.ac_mut().spinner = None;

        // Finalize any streaming assistant message.
        self.ac_mut().master_session.chat.finalize_assistant();

        // Show abort status.
        self.ac_mut()
            .master_session
            .chat
            .add_entry(ChatEntry::Status {
                text: "Operation aborted".to_string(),
            });
    }
}
