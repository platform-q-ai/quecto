use super::app_selection::{SelectionAnchor, apply_selection_highlight};
use super::*;
use crate::application::session_payloads::{self, ResumedChatMessage};
use crate::interface::select_overlay::{
    build_resume_selector_overlay, build_rewind_selector_overlay,
};
use crate::interface::theme;

/// Format a Unix timestamp as `YYYY-MM-DD HH:MM` in **local** time, falling
/// back to UTC if the platform's local-time conversion is unavailable.
fn format_unix_minutes(secs: u64) -> String {
    format_local_minutes(secs).unwrap_or_else(|| format_utc_minutes(secs))
}

/// Local time via `libc::localtime_r`. Returns `None` if the conversion fails.
fn format_local_minutes(secs: u64) -> Option<String> {
    let t = secs as libc::time_t;
    // SAFETY: `libc::tm` is plain-old-data; an all-zero value is a valid initial state for libc to fill.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: `&t`/`&mut tm` point to live locals; localtime_r fills `tm` and returns null on failure (checked next).
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
        return None;
    }
    Some(format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min
    ))
}

/// UTC fallback (pure arithmetic) when local-time conversion is unavailable.
pub(super) fn format_utc_minutes(secs: u64) -> String {
    let secs = secs as i64;
    let days = secs.div_euclid(86_400);
    let mut rem = secs.rem_euclid(86_400);
    let hour = rem / 3_600;
    rem %= 3_600;
    let minute = rem / 60;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}")
}

pub(super) fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    (year, m as u32, d as u32)
}

impl App {
    // ── Slash command handlers ─────────────────────────────────────────

    pub(super) fn reject_unknown_slash_command(&mut self, command: &str) {
        self.chat.add_entry(ChatEntry::Status {
            text: format!(
                "Unknown slash command: {command}\nType /help to see available commands."
            ),
        });
        self.notify("Unknown slash command", NotifyLevel::Warning);
    }

    pub(super) fn show_help(&mut self) {
        let mut text = String::from(
            "Keyboard shortcuts:\n\
             \x20 Enter          Send message\n\
             \x20 Shift+Enter    Insert newline\n\
             \x20 Alt+Enter      Insert newline\n\
             \x20 Escape         Abort agent / clear editor\n\
             \x20 Esc Esc        Choose a previous turn to go back to\n\
             \x20 Ctrl+C         Clear editor first, abort if empty\n\
             \x20 Ctrl+D         Exit\n\
             \x20 Ctrl+L         Open model selector\n\
             \x20 Ctrl+O         Toggle tool output expansion\n\
             \x20 Ctrl+Shift+A   Toggle workflow auto-continue\n\
             \x20 Ctrl+Shift+N   Toggle workflow completion nudge\n\
             \x20 Ctrl+Z         Suspend (resume with fg)\n\
             \x20 PageUp/Down    Scroll chat\n\
             \x20 Up/Down        Input history\n\
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
        self.chat.add_entry(ChatEntry::Status { text });
    }

    pub(super) fn show_workflow_status(&mut self) {
        let wf = &self.workflow_bar;
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
        self.chat.add_entry(ChatEntry::Status { text });
    }

    pub(super) fn toggle_workflow_auto_continue(&mut self) {
        let next = !self.workflow_auto_continue;
        self.send_command(Command::SetWorkflowAutomation {
            id: Some("workflow-auto".into()),
            auto_continue: Some(next),
            completion_nudge: None,
        });
    }

    pub(super) fn toggle_workflow_completion_nudge(&mut self) {
        let next = !self.workflow_completion_nudge;
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
        if let Some((used, window)) = stats.context_usage {
            self.footer.update_context_usage(used, window);
            self.context_stats_requested = true;
        }
        self.footer.set_cost(if stats.cost > 0.0 {
            Some(stats.cost)
        } else {
            None
        });
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
        self.chat.add_entry(ChatEntry::Status {
            text: format!(
                "Session: {} | Messages: {} | Tokens: ↑{} ↓{} | Cost: ${:.4}",
                stats.session_key,
                stats.total_messages,
                stats.input_tokens,
                stats.output_tokens,
                stats.cost
            ),
        });
    }

    pub(super) fn send_set_model(&mut self, model: &str) {
        self.send_command(Command::SetModel {
            id: Some("sm".into()),
            model: Some(model.to_string()),
            provider: None,
            model_id: None,
        });
        self.footer.set_model(model);
        self.current_model = Some(model.to_string());
        self.context_stats_requested = false;
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
            self.chat.add_entry(ChatEntry::Status {
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
        self.resume_selector = Some(SelectList::new(items, 10));
    }

    pub(super) fn handle_resume_selector_key(&mut self, key: &Key) {
        if let Some(selector) = &mut self.resume_selector {
            selector.handle_input(key);
            match selector.take_result() {
                SelectResult::Selected(session) => {
                    self.resume_selector = None;
                    self.send_resume_session(&session);
                }
                SelectResult::Cancelled => {
                    self.resume_selector = None;
                }
                SelectResult::Pending => {}
            }
        }
    }

    pub(super) fn replace_chat_with_messages(&mut self, data: &serde_json::Value) {
        let messages = match session_payloads::parse_resumed_messages(data) {
            Ok(messages) => messages,
            Err(error) => {
                let text = format!("Invalid resume payload: {}", error.description());
                self.chat
                    .add_entry(ChatEntry::Status { text: text.clone() });
                self.notify(&text, NotifyLevel::Error);
                return;
            }
        };

        self.chat.clear();
        for message in messages {
            match message {
                ResumedChatMessage::User(text) => self.chat.add_entry(ChatEntry::User { text }),
                ResumedChatMessage::Assistant(text) => self.chat.add_entry(ChatEntry::Assistant {
                    text,
                    streaming: false,
                }),
            }
        }
        self.chat.add_entry(ChatEntry::Status {
            text: "Session resumed".to_string(),
        });
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

    /// Build the below-chat section (sub-agent panel → workflow bar → spinner →
    /// autocomplete → editor → notifications → footer). This is
    /// the region whose height changes reflow the chat, so the headless harness
    /// captures it directly to assert on layout stability.
    pub(super) fn compose_bottom(&mut self, width: usize) -> Vec<String> {
        let mut workflow_bar_state = self.workflow_bar.clone();
        workflow_bar_state.workflow_auto_continue = self.workflow_auto_continue;
        workflow_bar_state.workflow_completion_nudge = self.workflow_completion_nudge;

        let mut bottom = Vec::new();

        // Widgets above editor (subagent bars stay on top, visible).
        bottom.extend(self.widgets_above.render(width));

        // Quecto-style workflow widget above the editor.
        bottom.extend(workflow_bar::render_widget(&workflow_bar_state, width));

        // Spinner sits between widgets_above and autocomplete (#534). While
        // sub-agents are tracked, RESERVE its line (spinner when active, blank
        // when idle): workflow children fire notifications that make the parent
        // do many short runs, each creating/dropping the spinner — a toggling
        // 0↔1 line would reflow the chat on every run (the panel-size 6↔7 /
        // 11↔12 judder). A reserved slot keeps the below-chat height stable.
        if let Some(spinner) = &mut self.spinner {
            bottom.extend(spinner.render(width));
        } else if !self.subagent_local.is_empty() {
            // Parent is idle but sub-agents are tracked. Keep the reserved slot
            // meaningful: if any child is still working, show an animated
            // "N working" indicator (so activity stays visible while the parent
            // waits); otherwise a blank keeps the height stable.
            let active = self
                .subagent_local
                .values()
                .filter(|t| subagent_status_is_active(&t.info.status))
                .count();
            if active > 0 {
                bottom.push(subagent_activity_line(active, self.subagent_frame));
            } else {
                bottom.push(String::new());
            }
        }

        // Autocomplete dropdown (slash commands, then @files — only one active).
        bottom.extend(self.autocomplete.render(width));
        bottom.extend(self.files_autocomplete.render(width));
        // Editor.
        bottom.extend(self.editor.render(width));
        // Notifications.
        bottom.extend(self.notifications.render(width));
        // Footer.
        bottom.extend(self.footer.render(width));

        bottom
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
        let full_width = self.terminal.width;
        let height = self.terminal.height;

        // A persistent left panel (#800) splits the screen horizontally once a
        // sub-agent exists: the body renders into the reduced right column and
        // the panel cell is prefixed onto each row afterward.
        let panel_visible = self.subagent_panel_visible();
        let panel_width = if panel_visible {
            SUBAGENT_PANEL_WIDTH.min(full_width / 2)
        } else {
            0
        };
        let width = full_width - panel_width;

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

        // Chat — render into available space above the bottom section.
        // Reserve MIN_CHAT_GAP lines for spacing between chat and editor (#480).
        const MIN_CHAT_GAP: usize = 3;
        let chat_height = height.saturating_sub(bottom_height + 2 + MIN_CHAT_GAP);
        let chat = self.active_chat_mut();
        chat.set_viewport_height(chat_height);
        let mut chat_lines = chat.render(width);

        // If chat is taller than available space, show only the tail (auto-scroll).
        if chat_lines.len() > chat_height {
            let start = chat_lines.len() - chat_height;
            chat_lines = chat_lines[start..].to_vec();
        }
        lines.extend(chat_lines);

        // Pad between chat and bottom to push bottom to the screen bottom.
        let available = height.saturating_sub(bottom_height);
        while lines.len() < available {
            lines.push(String::new());
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
        if let Some(selector) = &mut self.resume_selector {
            let (selector_lines, overlay_width) =
                build_resume_selector_overlay(selector, width, height);
            Self::composite_centered(&mut lines, &selector_lines, overlay_width, width, height);
        }
        if let Some(selector) = &mut self.rewind_selector {
            let (selector_lines, overlay_width) =
                build_rewind_selector_overlay(selector, width, height);
            Self::composite_centered(&mut lines, &selector_lines, overlay_width, width, height);
        }
        if let Some(selector) = &mut self.model_selector {
            let overlay_width = width.saturating_sub(4).min(60);
            let selector_lines = selector.render(overlay_width);
            Self::composite_centered(&mut lines, &selector_lines, overlay_width, width, height);
        }

        // Enforce width on every (body) line.
        for line in &mut lines {
            if crate::interface::utils::visible_width(line) > width {
                *line = crate::interface::utils::truncate_to_width(line, width, None);
            }
        }

        // Prefix the persistent left panel onto each row (#800). Panel cells are
        // pre-padded to exactly `panel_width` visible columns, so concatenation
        // yields full-width rows.
        if panel_visible {
            let panel = self.render_subagent_panel(panel_width, height);
            for (i, line) in lines.iter_mut().enumerate() {
                let cell = panel
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| " ".repeat(panel_width));
                *line = format!("{cell}{line}");
            }
        }

        // Store rendered lines for text selection extraction (#528), but only
        // while a selection is (or was) active. The clean copy is consumed
        // exclusively by mouse text-selection extraction, so idle/streaming
        // frames must not deep-clone the whole screen buffer every tick (#757).
        // Must happen BEFORE highlight injection to avoid leaking
        // reverse-video escapes into the extraction buffer (#546 review).
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
                lines[row] = crate::interface::overlay::splice_line(
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
        let mut lines = self.compose_frame();
        // Diagnostic: dump the WHOLE frame (chat + below-chat) so a transient
        // line (too fast to see) can be replayed from the log.
        if let Some(path) = self.render_log_path.as_deref() {
            self.log_render_frame(path, &lines);
        }

        // Apply mouse selection highlight (#546) to the display copy only, so
        // the extraction buffer (`last_rendered_lines`) stays clean.
        apply_selection_highlight(&self.selection, &mut lines);

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
        self.send_new_session();
        self.chat.clear();
        self.footer.set_context(None, 0);
        self.context_stats_requested = false;
        self.notify(message, NotifyLevel::Success);
    }

    pub(super) fn send_new_session(&mut self) {
        self.send_command(Command::NewSession { id: None });
    }

    #[cfg(test)]
    pub(super) fn send_clear_history(&mut self) {
        self.send_command(Command::ClearHistory { id: None });
    }

    // ── Command sending ───────────────────────────────────────────────

    pub(super) fn send_command(&mut self, cmd: Command) {
        let mut sender = self.client.clone_sender();
        let failure_tx = self.command_send_failure_tx.clone();
        let command_kind = cmd.kind();
        tokio::spawn(async move {
            if let Err(e) = sender.send(&cmd).await {
                let _ = failure_tx
                    .send(CommandSendFailure {
                        command_kind,
                        error: e.to_string(),
                    })
                    .await;
            }
        });
    }

    pub(super) fn handle_command_send_failure(&mut self, failure: CommandSendFailure) {
        self.notify(
            &format!(
                "Failed to send {} command: {}",
                failure.command_kind, failure.error
            ),
            NotifyLevel::Error,
        );
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
        let mut result = String::new();

        for row in start.row..=end.row {
            let row_idx = row as usize;
            if row_idx >= lines.len() {
                break;
            }
            let visible = strip_ansi_for_selection(&lines[row_idx]);
            let chars: Vec<char> = visible.chars().collect();

            let col_start = if row == start.row {
                start.col as usize
            } else {
                0
            };
            let col_end = if row == end.row {
                end.col as usize
            } else {
                chars.len()
            };

            let col_start = col_start.min(chars.len());
            let col_end = col_end.min(chars.len());

            let segment: String = chars[col_start..col_end].iter().collect();

            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(&segment);
        }

        result
    }
}

/// Animated "N subagent(s) working…" line shown in the reserved spinner slot
/// while the parent is idle but children are still active.
pub(super) fn subagent_activity_line(active: usize, frame: usize) -> String {
    use crate::interface::theme::SPINNER_FRAMES;
    let spin = theme::spinner(SPINNER_FRAMES[frame % SPINNER_FRAMES.len()]);
    let noun = if active == 1 { "subagent" } else { "subagents" };
    format!(
        "  {} {}",
        spin,
        theme::muted(&format!("{active} {noun} working..."))
    )
}

/// Strip ANSI escape sequences (CSI + OSC) for the render-log diagnostic and
/// the headless test harness.
pub(super) fn strip_ansi(s: &str) -> String {
    crate::interface::ansi::strip_ansi(s)
}
