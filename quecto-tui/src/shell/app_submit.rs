use super::*;
use crate::conversation::image_attachments::with_image_markers;

impl App {
    pub(super) fn handle_submit(&mut self, text: &str) {
        let trimmed = text.trim();
        // Text, or images alone (#2425), makes a message.
        if trimmed.is_empty() && self.attachments.pending.is_empty() {
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
                    self.request_ordinary_exit();
                    return;
                }
                "/clear" => {
                    self.reset_session("Conversation cleared");
                    return;
                }
                "/new" => {
                    self.reset_workspace();
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
                        self.send_resume_session(session);
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
                _ if trimmed.strip_prefix("/image").is_some_and(|rest| {
                    rest.is_empty() || rest.starts_with(char::is_whitespace)
                }) =>
                {
                    self.attach_image_file(&trimmed["/image".len()..]);
                    return;
                }
                "/refresh-models" => {
                    self.send_refresh_models();
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
                _ if trimmed.strip_prefix("/setup").is_some_and(|rest| {
                    rest.is_empty() || rest.starts_with(char::is_whitespace)
                }) =>
                {
                    // #2024 S6: the TUI only composes the walkthrough text; the
                    // agent reads the docs pages and asks before writing.
                    match crate::setup::SetupCommand::parse(&trimmed["/setup".len()..]) {
                        // Setup targets the master session and its files —
                        // never a focused child (possibly in a container).
                        crate::setup::SetupCommand::Walkthrough(_)
                            if self.ac().roster.active_agent_id.is_some() =>
                        {
                            self.notify(
                                crate::setup::SETUP_FROM_MASTER,
                                crate::components::notification::NotifyLevel::Error,
                            )
                        }
                        crate::setup::SetupCommand::Walkthrough(area) => {
                            // The walkthrough is the TUI's own prompt: it
                            // never takes the user's attached images (#2425).
                            let prompt = crate::setup::setup_walkthrough_prompt(&area);
                            self.send_user_message(&prompt, false);
                        }
                        crate::setup::SetupCommand::Usage => self.notify(
                            crate::setup::SETUP_USAGE,
                            crate::components::notification::NotifyLevel::Warning,
                        ),
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

        self.send_user_message(text, true);
    }

    /// Send `text` as a user message to the active session, with the
    /// attached images when `with_images`: a message the user composed,
    /// never a slash command's own prompt (#2425). The images leave the
    /// composer only when the message is enqueued.
    fn send_user_message(&mut self, text: &str, with_images: bool) {
        let images = match with_images {
            true => self.attachments.pending.attachments(),
            false => Vec::new(),
        };
        let shown = with_image_markers(text, images.len());
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
            let cmd = user_message_command(text, images, self.active_subagent_running());
            if !self.message_fits_one_frame(&cmd, text) {
                return;
            }
            // Append to the sub-agent transcript ONLY when the route actually
            // enqueued it (#804 review): a failed route (no live sender / full
            // channel) never delivered the prompt, so a User entry would diverge
            // UI from state. The chips go with the message, and stay when it
            // was not delivered (#2425).
            if self.send_to_active_subagent(cmd) {
                if with_images {
                    self.attachments.pending.clear();
                }
                self.active_chat_mut()
                    .add_entry_follow_tail(ChatEntry::User { text: shown });
            } else {
                // Failed route (no live sender / full channel): the user must
                // see the message was not delivered (fix pass item 5).
                self.note_subagent_undeliverable(&agent_id, "unattached");
            }
            return;
        }

        let cmd = user_message_command(text, images, self.ac().agent_state.is_running());
        if !self.message_fits_one_frame(&cmd, text) {
            return;
        }
        // The composed text always lands in the chat (the editor was
        // already emptied by take_submit) — on a dead connection it is the
        // only surviving copy (#1470 r3/r6, single add site).
        self.ac_mut()
            .master_session
            .chat
            .add_entry_follow_tail(ChatEntry::User { text: shown });
        // Refuse when the connection is known dead (#1470): the writer
        // channel can outlive the stream, so an enqueue could "succeed" and
        // the message silently vanish. The persistent refusal Status line
        // keeps the undelivered message diagnosable after the toast expires.
        // The chips stay for a later send (#2425).
        if !self.ac().agent_connected {
            self.note_disconnected_refusal();
            return;
        }
        if self.send_command(cmd) && with_images {
            self.attachments.pending.clear();
        }
    }

    /// Whether `cmd` fits one protocol frame (#2425). One that does not would
    /// be dropped by the writer unseen, so it is refused here instead: a
    /// notice, its text back in the editor, its chips kept.
    fn message_fits_one_frame(&mut self, cmd: &Command, text: &str) -> bool {
        match cmd.fits_one_frame() {
            true => true,
            false => {
                self.editor.set_text(text);
                let cap = quecto_line_io::PROTOCOL_LINE_CAP_BYTES / (1024 * 1024);
                let notice = format!(
                    "Message not sent: it is over the {cap} MiB one message can carry; \
                     remove an image or shorten the text"
                );
                self.notify(&notice, crate::components::notification::NotifyLevel::Error);
                false
            }
        }
    }

    /// Surface an undeliverable sub-agent message (#1466 fix pass item 5):
    /// a persistent Status line plus a toast, so the drop is never silent.
    fn note_subagent_undeliverable(&mut self, agent_id: &str, status: &str) {
        let text = format!("Message not delivered — sub-agent '{agent_id}' is {status}.");
        self.active_chat_mut()
            .add_entry(ChatEntry::Status { text: text.clone() });
        self.notify(&text, crate::components::notification::NotifyLevel::Warning);
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
                if let Some(entry) = self.ac_mut().roster.tracked.get_mut(&id) {
                    let mut info = entry.info.clone();
                    info.status = "idle".to_string();
                    entry.update_info_at(info, tokio::time::Instant::now());
                }
            }
            return;
        }

        self.send_command(Command::Abort { id: None });

        // Abort the state machine — does NOT set running false; the matched
        // AgentEnd arrives and guards against stale events corrupting state (#502).
        self.ac_mut().agent_state.abort();
        self.ac_mut()
            .stop_coordinator_clock(tokio::time::Instant::now());
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

/// A user message with its `images` (#2425): a follow-up while the session
/// runs, else a prompt.
fn user_message_command(
    text: &str,
    images: Vec<quecto_image::ImageAttachment>,
    running: bool,
) -> Command {
    match running {
        true => Command::FollowUp {
            id: None,
            message: text.to_string(),
            images,
        },
        false => Command::Prompt {
            id: None,
            message: text.to_string(),
            streaming_behavior: None,
            images,
        },
    }
}

#[cfg(test)]
#[path = "app_submit_setup_tests.rs"]
mod app_submit_setup_tests;
