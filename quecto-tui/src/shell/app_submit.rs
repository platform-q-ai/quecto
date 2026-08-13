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
                    self.reset_session("New session started");
                    return;
                }
                "/tab-new" => {
                    let tab = self.open_placeholder_tab(None);
                    self.notify(
                        &format!("Opened tab {}", tab.0),
                        crate::components::notification::NotifyLevel::Info,
                    );
                    return;
                }
                "/tab-close" => {
                    let tab = self.active_tab;
                    match self.close_tab(tab, false) {
                        Ok(_) => self.notify(
                            &format!("Closed tab {} (agent detached)", tab.0),
                            crate::components::notification::NotifyLevel::Info,
                        ),
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
                    self.send_resume_session(session);
                    return;
                }
                _ if trimmed.starts_with("/effort") => {
                    let arg = trimmed["/effort".len()..].trim();
                    self.handle_effort_command(arg);
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
        if self.ac().roster.active_agent_id.is_some() {
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
            }
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
