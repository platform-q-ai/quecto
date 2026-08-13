pub(super) use super::app_render_helpers::{
    strip_ansi, subagent_activity_line, subagent_idle_line,
};
use super::app_selection::{SelectionAnchor, apply_selection_highlight, display_col_to_char_idx};
use super::*;
use crate::components::select_list::route_overlay_key;
use crate::components::select_overlay::{
    build_resume_selector_overlay, build_rewind_selector_overlay, build_select_overlay,
};
use crate::components::theme;
use crate::protocol::session_payloads;

// Wall-clock formatting helpers live in `app_time` (this module is at the
// source line cap); re-exported so `app_methods::format_utc_minutes` and the
// internal `format_unix_minutes` call sites stay put.
use super::app_time::format_unix_minutes;
// Only the unit tests reference these through `app_methods::…`; production reads
// go straight to `app_time` (via `format_unix_minutes`), so gate the re-export.
#[cfg(test)]
pub(super) use super::app_time::{civil_from_days, format_utc_minutes};

impl App {
    // ── Slash command handlers ─────────────────────────────────────────

    pub(super) fn reject_unknown_slash_command(&mut self, command: &str) {
        self.master_session.chat.add_entry(ChatEntry::Status {
            text: format!(
                "Unknown slash command: {command}\nType /help to see available commands."
            ),
        });
        self.notify("Unknown slash command", NotifyLevel::Warning);
    }

    pub(super) fn show_help(&mut self) {
        let mut text = String::from(
            "Keyboard shortcuts:\n\
             \x20 Enter          Send message (idle) / queue follow-up (running)\n\
             \x20 Shift+Enter    Insert newline\n\
             \x20 Alt+Enter      Insert newline\n\
             \x20 Escape         Abort/cancel running agent or clear editor\n\
             \x20 Esc Esc        Choose a previous turn to go back to\n\
             \x20 Ctrl+C         Clear editor first, abort if empty\n\
             \x20 Ctrl+D         Exit\n\
             \x20 Ctrl+L         Open model selector\n\
             \x20 Ctrl+T         Open tool policy selector\n\
             \x20 Ctrl+O         Toggle tool output expansion\n\
             \x20 Ctrl+Shift+A   Toggle workflow auto-continue\n\
             \x20 Ctrl+Shift+N   Toggle workflow completion nudge\n\
             \x20 Ctrl+Z         Suspend (resume with fg)\n\
             \x20 PageUp/Down    Scroll chat\n\
             \x20 Up/Down        Input history\n\
             \n\
             Mouse / links:\n\
             \x20 Wheel          Scroll chat\n\
             \x20 Drag           Select text\n\
             \x20 Shift+click    Open markdown/OSC 8 link in browser\n\
             \x20                (when mouse capture is on; some terminals\n\
             \x20                use Ctrl/Cmd+click instead)\n\
             \n\
             Slash commands:",
        );
        // Derive the slash-command listing from the single source of truth so it
        // can never drift from the autocomplete set or the dispatch handler.
        for command in builtin_commands() {
            text.push_str(&format!(
                "\n  /{:<14} {}",
                command.name, command.description
            ));
        }
        self.master_session
            .chat
            .add_entry(ChatEntry::Status { text });
    }

    pub(super) fn show_workflow_status(&mut self) {
        let wf = &self.master_session.workflow_bar;
        let text = if workflow_bar::render_widget(wf, self.terminal.width).is_empty() {
            "Workflow is not active. Start quecto-tui with --workflow to enable it.".to_string()
        } else {
            let current = wf
                .current_step_id()
                .map(|id| {
                    format!(
                        "next step {id}: {}",
                        sanitize_workflow_status_text(wf.current_step_label().unwrap_or(""), 80)
                    )
                })
                .unwrap_or_else(|| "complete".to_string());
            format!(
                "Workflow status: {}/{} ({current})",
                wf.done,
                wf.total.max(1)
            )
        };
        self.master_session
            .chat
            .add_entry(ChatEntry::Status { text });
    }

    pub(super) fn toggle_workflow_auto_continue(&mut self) {
        let next = !self.workflow.auto_continue;
        self.send_command(Command::SetWorkflowAutomation {
            id: Some("workflow-auto".into()),
            auto_continue: Some(next),
            completion_nudge: None,
        });
    }

    pub(super) fn toggle_workflow_completion_nudge(&mut self) {
        let next = !self.workflow.completion_nudge;
        self.send_command(Command::SetWorkflowAutomation {
            id: Some("workflow-nudge".into()),
            auto_continue: None,
            completion_nudge: Some(next),
        });
    }

    pub(super) fn send_session_stats(&mut self) {
        self.send_command(Command::GetSessionStats {
            id: Some("stats".into()),
        });
    }

    /// Request session stats for a quiet footer-only refresh (no chat Status
    /// line). Routed by the "stats-footer" id in the response handler.
    pub(super) fn send_session_stats_footer(&mut self) {
        self.send_command(Command::GetSessionStats {
            id: Some("stats-footer".into()),
        });
    }

    /// Update the footer's context/cost indicators from a session-stats
    /// payload without emitting a chat entry.
    pub(super) fn update_footer_stats(&mut self, data: &serde_json::Value) {
        let stats = session_payloads::parse_session_stats(data);
        if stats.context_usage.is_some() {
            self.sessions.context_stats_requested = true;
        }
        // Shared session-stats→footer mapping (context + cost gate); see #805.
        self.master_session.footer.apply_session_stats(&stats);
    }

    pub(super) fn send_list_sessions(&mut self) {
        self.send_command(Command::ListSessions {
            id: Some("resume-list".into()),
        });
    }

    pub(super) fn send_resume_session(&mut self, session: &str) {
        if session.trim().is_empty() {
            self.send_list_sessions();
            return;
        }
        self.send_command(Command::ResumeSession {
            id: Some("resume".into()),
            session: session.trim().to_string(),
        });
    }

    pub(super) fn show_session_stats(&mut self, data: &serde_json::Value) {
        // Footer context/cost update has a single owner; this adds the chat line.
        self.update_footer_stats(data);
        let stats = session_payloads::parse_session_stats(data);
        self.master_session.chat.add_entry(ChatEntry::Status {
            text: format!(
                "Session: {} | Messages: {} | Tokens: ↑{} ↓{}",
                stats.session_key, stats.total_messages, stats.input_tokens, stats.output_tokens
            ),
        });
    }

    // ── Resume selector ─────────────────────────────────────────────

    pub(super) fn open_resume_selector(&mut self, data: &serde_json::Value) {
        let sessions = session_payloads::parse_resume_sessions(data);
        if sessions.is_empty() {
            let text = if session_payloads::has_session_entries(data) {
                "No resumable CLI sessions found."
            } else {
                "No persisted sessions found."
            };
            self.master_session.chat.add_entry(ChatEntry::Status {
                text: text.to_string(),
            });
            return;
        }
        let items = sessions
            .into_iter()
            .map(|session| {
                let when = session
                    .updated_unix_secs
                    .map(format_unix_minutes)
                    .unwrap_or_else(|| "unknown time".to_string());
                SelectItem {
                    value: session.key,
                    label: session.title,
                    description: Some(format!("{when}   ({} msgs)", session.message_count)),
                }
            })
            .collect::<Vec<_>>();
        self.sessions.resume_selector = Some(SelectList::new(items, 10));
    }

    pub(super) fn handle_resume_selector_key(&mut self, key: &Key) {
        if let Some(session) = route_overlay_key(&mut self.sessions.resume_selector, key) {
            self.send_resume_session(&session);
        }
    }

    pub(super) fn replace_chat_with_messages(&mut self, data: &serde_json::Value) {
        self.replace_chat_with_messages_with_empty_status(data, "Session resumed");
    }

    pub(super) fn replace_chat_with_messages_with_empty_status(
        &mut self,
        data: &serde_json::Value,
        empty_status: &str,
    ) -> bool {
        let messages = match session_payloads::parse_resumed_messages(data) {
            Ok(messages) => messages,
            Err(error) => {
                let text = format!("Invalid resume payload: {}", error.description());
                self.master_session
                    .chat
                    .add_entry(ChatEntry::Status { text: text.clone() });
                self.notify(&text, NotifyLevel::Error);
                return false;
            }
        };

        let has_displayable_messages = !messages.is_empty();
        self.master_session.chat.clear();
        for entry in Self::resumed_chat_entries(messages) {
            self.master_session.chat.add_entry(entry);
        }
        if !has_displayable_messages {
            self.master_session.chat.add_entry(ChatEntry::Status {
                text: empty_status.to_string(),
            });
        }
        has_displayable_messages
    }

    // ── Notifications ─────────────────────────────────────────────────

    pub(super) fn notify(&mut self, message: &str, level: NotifyLevel) {
        self.notifications.push(Notification::new(message, level));
    }

    // ── Rendering ─────────────────────────────────────────────────────

    /// Diagnostic: append one frame (ANSI-stripped) to the render log.
    fn log_render_frame(&self, path: &str, bottom: &[String]) {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        // Frames can contain conversation/tool content, so create owner-only
        // (0600) and refuse to follow a pre-planted symlink (O_NOFOLLOW) — this
        // is a diagnostic that may run on a shared host.
        let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
        else {
            return;
        };
        let _ = writeln!(f, "=== frame: {} below-chat lines ===", bottom.len());
        for line in bottom {
            let _ = writeln!(f, "  | {}", strip_ansi(line));
        }
    }

    /// Build the below-chat section (spinner → autocomplete → editor →
    /// notifications → footer). The sub-agent and workflow bars moved out of this
    /// stack under the sub-agent-first layout (#820). This is the region whose
    /// height changes reflow the chat, so the headless harness captures it
    /// directly to assert on layout stability.
    pub(super) fn compose_bottom(&mut self, width: usize) -> Vec<String> {
        let mut bottom = Vec::new();

        // Sub-agent/workflow bars moved out of the bottom stack.
        if self.subagents.active_agent_id.is_none() && self.spinner.is_some() {
            // Master is active and mid-turn: show its richer tool spinner (tool
            // name + elapsed), the only master-local render telemetry layered on
            // top of the shared per-session `running` flag (#828).
            if let Some(spinner) = &mut self.spinner {
                if self.subagents.tracked.is_empty() {
                    bottom.push(String::new());
                }
                bottom.extend(spinner.render(width));
            }
        } else if self.active_subagent_running() {
            // The active session is mid-turn (a sub-agent processing queued
            // follow-up work, or the master before its spinner exists); show the
            // working indicator so it never looks dead.
            bottom.push(String::new());
            bottom.push(subagent_activity_line(1, self.subagents.frame));
        } else if !self.subagents.tracked.is_empty() {
            let active = self.subagents.tracked_active_count();
            if active > 0 {
                bottom.push(subagent_activity_line(active, self.subagents.frame));
            } else {
                bottom.push(subagent_idle_line(self.subagents.tracked.len()));
            }
        }

        // Autocomplete dropdown (slash commands, then @files — only one active).
        bottom.extend(self.autocomplete.render(width));
        bottom.extend(self.workspace.files_autocomplete.render(width));
        // Editor. Hide the block cursor while the sub-agent panel has focus so
        // it's unambiguous that keystrokes won't land in the input.
        self.editor
            .set_show_cursor(!matches!(self.subagents.focus, Focus::Panel));
        bottom.extend(self.editor.render(width));
        // Notifications.
        bottom.extend(self.notifications.render(width));
        // Footer — render the ACTIVE session's gauges (master's own, or the
        // selected sub-agent's context-window / cost / model), so a selected
        // sub-agent shows ITS usage rather than the master's (#805).
        bottom.extend(self.active_footer_render(width));

        bottom
    }

    /// The current frame's horizontal split: `(panel_width, divider_width,
    /// body_width)`. The persistent left panel is always on once connected
    /// (#820); `compose_frame` and the headless harness both derive widths from
    /// here so the harness reproduces the exact body width the user sees.
    pub(super) fn frame_split(&self) -> (usize, usize, usize) {
        let full_width = self.terminal.width;
        let panel_visible = self.subagent_panel_visible();
        let panel_width = if panel_visible {
            SUBAGENT_PANEL_WIDTH.min(full_width / 2)
        } else {
            0
        };
        // Two columns when visible: the focus-highlighted vertical divider (#802)
        // plus a one-space gutter so the main pane isn't flush against the bar.
        let divider_width = if panel_visible { 2 } else { 0 };
        (
            panel_width,
            divider_width,
            full_width - panel_width - divider_width,
        )
    }

    /// The reduced body width the chat/bottom stack render into (#820 review).
    #[cfg(any(test, feature = "test-harness"))]
    pub(super) fn body_width(&self) -> usize {
        self.frame_split().2
    }

    /// Build the full screen frame (chat + bottom section + overlays), clean
    /// (pre-selection-highlight) and width-enforced, WITHOUT writing it.
    /// `render()` writes the result; the headless harness (`tui_harness`)
    /// captures it for layout/flicker assertions without a terminal.
    ///
    /// Contract: composition must be **render-idempotent** — calling it twice in
    /// a row yields the same frame. Its only side effects are render-state
    /// (`set_viewport_height`, `last_rendered_lines`), never external I/O or
    /// model mutation. The harness relies on this (it composes per capture); a
    /// future non-idempotent step would make captures diverge from real renders.
    pub(super) fn compose_frame(&mut self) -> Vec<String> {
        let height = self.terminal.height;

        // A persistent left panel (#800/#820) splits the screen horizontally:
        // the body renders into the reduced right column and the panel cell is
        // prefixed onto each row afterward.
        let panel_visible = self.subagent_panel_visible();
        let (panel_width, _divider_width, width) = self.frame_split();

        // Sample the wall clock ONCE per frame and thread it through every
        // elapsed-timer render path (panel rows, Master uptime, main-pane
        // title). compose_frame is contractually render-idempotent, so the
        // clock must not be re-sampled deeper in the call tree (#820 review).
        let now = tokio::time::Instant::now();

        let mut lines = Vec::new();

        // ── Render bottom section first to know its height ──────────
        let bottom = self.compose_bottom(width);
        let bottom_height = bottom.len();

        // ── Render top section (header + chat) ──────────────────────
        // Header.
        let version = env!("CARGO_PKG_VERSION");
        lines.push(theme::dim(&format!(
            "quecto-tui v{} — Enter send, Shift+Enter newline, /help for commands",
            version
        )));

        // Sub-agent-first main pane (#820 / #1288 / #1309): title + optional
        // compact workflow progress framed by separator rules above the chat.
        // Phase pills / shortcut hints stay omitted (#1246).
        let main_box_width = width;
        let main_pane_workflow = self.render_main_pane_workflow(width, main_box_width, now);
        lines.extend(main_pane_workflow);

        // Chat uses all space above bottom, with one blank separator above the
        // streaming area so it does not sit tight against the master idle
        // timer/title area (#1323). The bottom stack already reserves the blank
        // separator before the working spinner. Top-pad short transcripts so the
        // latest output sits directly above that lower separator.
        let top_chrome_height = lines.len();
        let unpadded_chat_height = height.saturating_sub(bottom_height + top_chrome_height);
        let streaming_vertical_padding = usize::from(unpadded_chat_height > 1);
        let chat_height = unpadded_chat_height.saturating_sub(streaming_vertical_padding);
        // Environment selected (#1369 follow-up): the main pane body is
        // container info ONLY — no parent/agent transcript may render beneath
        // the environment chrome, so the conversation is suppressed entirely.
        // Overflow: the conversation shows its tail (auto-scroll); the
        // environment body head-anchors (#1401 review, `clamp_environment_body`).
        let mut chat_lines = match self.render_environment_body(width) {
            Some(body) => Self::clamp_environment_body(body, chat_height),
            None => {
                let chat = self.active_chat_mut();
                chat.set_viewport_height(chat_height);
                let mut lines = chat.render(width);
                if lines.len() > chat_height {
                    lines = lines[lines.len() - chat_height..].to_vec();
                }
                lines
            }
        };
        while chat_lines.len() < chat_height {
            chat_lines.insert(0, String::new());
        }
        lines.push(String::new());
        lines.extend(chat_lines);

        let available = height.saturating_sub(bottom_height);
        while lines.len() < available {
            lines.insert(top_chrome_height + 1, String::new());
        }

        // ── Append bottom section ───────────────────────────────────
        lines.extend(bottom);

        // Final safety: ensure exactly `height` lines.
        if lines.len() > height {
            let start = lines.len() - height;
            lines = lines[start..].to_vec();
        }
        while lines.len() < height {
            lines.push(String::new());
        }

        // Composite the active centered overlay (only one is ever active at a
        // time). All three splice through the same ANSI-aware helper so the
        // centering and escape-safe splice rule lives in one place.
        if let Some(selector) = &mut self.sessions.resume_selector {
            let (selector_lines, overlay_width) =
                build_resume_selector_overlay(selector, width, height);
            Self::composite_centered(&mut lines, &selector_lines, overlay_width, width, height);
        }
        if let Some(selector) = &mut self.rewind.selector {
            let (selector_lines, overlay_width) =
                build_rewind_selector_overlay(selector, width, height);
            Self::composite_centered(&mut lines, &selector_lines, overlay_width, width, height);
        }
        if let Some(modal) = &mut self.tool_policy_modal {
            let (selector_lines, overlay_width) =
                crate::components::selectable_item_modal::build_selectable_item_modal_overlay(
                    "Tool Policy",
                    "Ctrl+Shift+A allow all • Ctrl+Shift+D disable matches • New agents only",
                    modal,
                    width,
                    height,
                );
            Self::composite_centered(&mut lines, &selector_lines, overlay_width, width, height);
        }
        if let Some(selector) = &mut self.inference.model_selector {
            let (selector_lines, overlay_width) =
                build_select_overlay(width, height, |content_width| {
                    selector.render(content_width)
                });
            Self::composite_centered(&mut lines, &selector_lines, overlay_width, width, height);
        }
        if let Some(selector) = &mut self.inference.effort_selector {
            let (selector_lines, overlay_width) =
                build_select_overlay(width, height, |content_width| {
                    selector.render(content_width)
                });
            Self::composite_centered(&mut lines, &selector_lines, overlay_width, width, height);
        }

        for line in &mut lines {
            if crate::components::utils::visible_width(line) > width {
                *line = crate::components::utils::truncate_to_width(line, width, None);
            }
        }

        if panel_visible {
            let panel = self.render_subagent_panel(panel_width, height, now);
            let divider = if matches!(self.subagents.focus, Focus::Panel) {
                theme::accent("│")
            } else {
                theme::dim("│")
            };
            for (i, line) in lines.iter_mut().enumerate() {
                let cell = panel
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| " ".repeat(panel_width));
                *line = format!("{cell}{divider} {line}");
            }
        }

        // Store rendered lines for text selection extraction (#528).
        if self.selection.is_some() {
            self.last_rendered_lines = lines.clone();
        } else {
            self.last_rendered_lines.clear();
        }
        lines
    }

    /// Splice a centered overlay into the frame `lines`, in place.
    ///
    /// Centers `overlay_lines` (clamped to leave a 4-row margin) and splices
    /// each row through the ANSI-aware splice helper so escape codes
    /// from the underlying frame can't bleed into or out of the overlay. Shared
    /// by every centered overlay (resume / rewind / model selectors).
    pub(super) fn composite_centered(
        lines: &mut [String],
        overlay_lines: &[String],
        overlay_width: usize,
        width: usize,
        height: usize,
    ) {
        let overlay_height = overlay_lines.len().min(height.saturating_sub(4));
        let start_row = height.saturating_sub(overlay_height) / 2;
        let start_col = width.saturating_sub(overlay_width) / 2;
        for i in 0..overlay_height {
            let row = start_row + i;
            if row < lines.len() && i < overlay_lines.len() {
                lines[row] = crate::components::overlay::splice_line(
                    &lines[row],
                    &overlay_lines[i],
                    start_col,
                    overlay_width,
                    width,
                );
            }
        }
    }

    /// Compose the current frame and write it to the terminal.
    pub(super) fn render(&mut self) {
        #[cfg(any(test, feature = "test-harness"))]
        {
            self.rendered_frames += 1;
            if self.suppress_paint {
                return;
            }
        }
        let mut lines = self.compose_frame();
        // Diagnostic: dump the WHOLE frame (chat + below-chat) so a transient
        // line (too fast to see) can be replayed from the log.
        if let Some(path) = self.render_log_path.as_deref() {
            self.log_render_frame(path, &lines);
        }

        // Apply mouse selection highlight (#546) to the display copy only, so
        // the extraction buffer (`last_rendered_lines`) stays clean. The body
        // begins after the optional sidepanel + divider, so clamp highlights to
        // that visible offset (#833).
        let (panel_width, divider_width, _body_width) = self.frame_split();
        let body_start_col = panel_width
            .saturating_add(divider_width)
            .min(u16::MAX as usize) as u16;
        apply_selection_highlight(&self.selection, &mut lines, body_start_col);

        // Write only changed terminal lines; the renderer tracks the previous
        // frame and performs a full draw on first use or after invalidation.
        if let Err(e) = self.renderer.render(&lines, self.terminal.width) {
            self.handle_render_failure(&e);
        }
    }

    pub(super) fn handle_render_failure(&mut self, error: &std::io::Error) {
        // A failed write/flush can leave the terminal cursor and synchronized
        // output state unknown. Do not trust the diff cache after that; force
        // the next successful frame to redraw from a known origin.
        self.renderer.invalidate();
        self.notify(
            &format!("Failed to render frame: {error}"),
            NotifyLevel::Error,
        );
    }

    pub(super) fn render_full(&mut self) {
        self.renderer.invalidate();
        self.terminal.clear_screen();
        self.render();
    }

    /// Reset the conversation — clears agent history, chat UI, and context display.
    pub(super) fn reset_session(&mut self, message: &str) {
        // Invalidate a pending off-loop disconnect diagnosis (#1470 r2/r3)
        // so the stale completion never lands in the fresh transcript.
        self.disconnect_diag_pending = false;
        // A dead connection still clears the LOCAL transcript (/clear must
        // work on a dead session) but reports honestly (#1470 r3).
        // Optimistic-enqueue window (#1470 r5): a just-died socket whose
        // Closed sentinel has not drained still enqueues successfully —
        // identical to pre-seam master; command acks are phase-2 scope.
        let was_connected = self.agent_connected;
        let agent_reset = self.send_new_session();
        self.master_session.chat.clear();
        // Invalidate in-flight ref recovery so a late get_message from the OLD
        // transcript can't splice into the cleared /clear-or-/new session (#1060 r4).
        self.clear_message_recovery();
        self.master_session.footer.set_context(None, 0);
        self.sessions.context_stats_requested = false;
        // The agent resets session-scoped state (e.g. the effort override, #1067)
        // on new_session; re-fetch so the footer tracks it (commands dispatch in
        // order, so this get_state observes the fresh session).
        self.send_state_resync();
        if agent_reset {
            self.notify(message, NotifyLevel::Success);
        } else if was_connected {
            // Connected but the enqueue failed (backpressure): a disconnect
            // diagnosis here would misdirect the user (#1470 r4).
            self.notify(
                "Cleared locally — sending new_session failed, retry /new",
                NotifyLevel::Warning,
            );
        } else {
            self.notify(
                "Cleared locally — agent disconnected, no new session started",
                NotifyLevel::Warning,
            );
        }
    }

    /// Request a fresh agent session; false when the connection is dead (#1470).
    pub(super) fn send_new_session(&mut self) -> bool {
        self.send_command(Command::NewSession { id: None })
    }

    #[cfg(test)]
    pub(super) fn send_clear_history(&mut self) {
        self.send_command(Command::ClearHistory { id: None });
    }

    // ── Command sending ───────────────────────────────────────────────

    pub(super) fn send_command(&mut self, cmd: Command) -> bool {
        // Refuse a known-dead connection: the writer channel outlives the closed
        // event stream, so `try_send` would return `Ok` and the command vanish
        // into a dead socket. Bail silently and return false — callers that show
        // user feedback (e.g. `reset_session`) act on that (#1470 review).
        if !self.agent_connected {
            // Roll back pending state (latching failed stub recalls, #1470
            // r4) and surface the refusal ONCE per disconnect episode —
            // per-command toasts would bury deliberate caller messaging.
            self.rollback_failed_history_command(MASTER_CONNECTION_ID, &cmd, true);
            if !self.disconnect_refusal_notified {
                self.disconnect_refusal_notified = true;
                self.notify(
                    "Agent disconnected — commands are not being sent",
                    NotifyLevel::Error,
                );
                // Notifications expire; a persistent transcript line keeps
                // later refusals diagnosable ("why won't history load?")
                // without per-command spam (#1470 r5).
                self.master_session.chat.add_entry(ChatEntry::Status {
                    text: "Agent disconnected — commands are not being sent (restart the TUI to reconnect)".to_string(),
                });
            }
            return false;
        }
        // Enqueue synchronously in call order onto the client's FIFO writer; a
        // prior per-command `tokio::spawn` let bursts reach the agent reordered
        // or look incomplete to an observer draining mid-batch (#1060 review).
        if let Err(e) = self.connection.try_send(&cmd) {
            // Roll back synchronously: the diagnostic side channel below is
            // best-effort, and if its receiver is gone we must not leave
            // pending history/resume/stub state stranded.
            self.rollback_failed_history_command(MASTER_CONNECTION_ID, &cmd, false);
            // Report without blocking the loop (a dropped notice is acceptable).
            let _ = self.command_send_failure_tx.try_send(CommandSendFailure {
                command: cmd,
                error: e.to_string(),
                connection: MASTER_CONNECTION_ID.to_string(),
            });
            return false;
        }
        true
    }

    // ── Mouse text selection (#528) ───────────────────────────────────

    /// Extract visible text from the rendered buffer between two selection anchors.
    pub(super) fn extract_selection(
        &self,
        start: &SelectionAnchor,
        end: &SelectionAnchor,
    ) -> String {
        // Normalize: ensure start ≤ end (top-to-bottom, left-to-right).
        let (start, end) = if (start.row, start.col) <= (end.row, end.col) {
            (start, end)
        } else {
            (end, start)
        };

        let lines = &self.last_rendered_lines;
        let (panel_width, divider_width, _) = self.frame_split();
        let body_start_col = panel_width.saturating_add(divider_width);
        let mut result = String::new();

        for row in start.row..=end.row {
            let row_idx = row as usize;
            if row_idx >= lines.len() {
                break;
            }
            let visible = strip_ansi_for_selection(&lines[row_idx]);
            let visible_width = crate::components::utils::visible_width(&visible);
            let chars: Vec<char> = visible.chars().collect();

            let col_start = if row == start.row {
                start.col as usize
            } else {
                0
            };
            let col_end = if row == end.row {
                end.col as usize
            } else {
                visible_width
            };

            let col_start = col_start.max(body_start_col).min(visible_width);
            let col_end = col_end.max(body_start_col).min(visible_width);

            let start_idx = display_col_to_char_idx(&chars, col_start);
            let end_idx = display_col_to_char_idx(&chars, col_end);
            let segment: String = chars[start_idx..end_idx].iter().collect();

            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(&segment);
        }

        result
    }
}
