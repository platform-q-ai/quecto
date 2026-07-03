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
                "/help" | "/hotkeys" => {
                    self.show_help();
                    return;
                }
                "/session" => {
                    self.send_session_stats();
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
        // steers THAT agent over its own connection (its dispatch loop queues
        // the prompt until its turn ends) and lands in its session, not master's.
        if self.active_agent_id.is_some() {
            let steer = self.active_subagent_running();
            let cmd = Command::Prompt {
                id: None,
                message: text.to_string(),
                streaming_behavior: steer.then(|| "steer".to_string()),
            };
            // Append to the sub-agent transcript ONLY when the route actually
            // enqueued it (#804 review): a failed route (no live sender / full
            // channel) never delivered the prompt, so a User entry would diverge
            // UI from state.
            if self.send_to_active_subagent(cmd) {
                self.active_chat_mut().add_entry(ChatEntry::User {
                    text: text.to_string(),
                });
            }
            return;
        }

        // Master session: add user message to chat and send to the primary agent.
        self.master_session.chat.add_entry(ChatEntry::User {
            text: text.to_string(),
        });
        let cmd = Command::Prompt {
            id: None,
            message: text.to_string(),
            streaming_behavior: if self.agent_state.is_running() {
                Some("steer".to_string())
            } else {
                None
            },
        };
        self.send_command(cmd);
    }

    // ── Abort handling (bug fix) ──────────────────────────────────────

    pub(super) fn handle_abort(&mut self) {
        // Abort targets the ACTIVE session (#802): a selected sub-agent's abort is
        // routed over its own connection and finalizes its transcript.
        if self.active_agent_id.is_some() {
            self.send_to_active_subagent(Command::Abort { id: None });
            self.active_chat_mut().finalize_assistant();
            if let Some(id) = self.active_agent_id.clone() {
                if let Some(session) = self.sessions.get_mut(&id) {
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
        self.agent_state.abort();
        self.master_session.footer.set_streaming(false);

        // Stop spinner / working indicator; `agent_state` stays aborting (#828).
        self.master_session.running = false;
        self.spinner = None;

        // Finalize any streaming assistant message.
        self.master_session.chat.finalize_assistant();

        // Show abort status.
        self.master_session.chat.add_entry(ChatEntry::Status {
            text: "Operation aborted".to_string(),
        });
    }
}
