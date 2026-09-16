pub(super) use super::app_render_helpers::{
    strip_ansi, subagent_activity_line, subagent_idle_line,
};
use super::app_selection::apply_selection_highlight;
use super::*;
use crate::components::select_list::route_overlay_key;
use crate::components::select_overlay::{
    build_resume_selector_overlay, build_rewind_selector_overlay, build_select_list_overlay,
    build_select_overlay,
};
use crate::components::theme;
use crate::protocol::session_payloads;
use crate::shell::app_session_stats_text;

use super::app_time::format_unix_minutes;

#[cfg(test)]
pub(super) use super::app_time::{civil_from_days, format_utc_minutes};

impl App {

    pub(super) fn reject_unknown_slash_command(&mut self, command: &str) {
        self.ac_mut()
            .master_session
            .chat
            .add_entry(ChatEntry::Status {
                text: format!(
                    "Unknown slash command: {command}\nType /help to see available commands."
                ),
            });
        self.notify("Unknown slash command", NotifyLevel::Warning);
    }

    /// Ctrl+Z: leave the alternate screen, stop the process, and restore raw
    /// mode + kitty protocol with a full repaint once resumed with `fg`.
    pub(super) fn suspend_and_resume(&mut self) {
        self.kitty.cleanup();
        self.terminal.show_cursor();
        crate::shell::signals::suspend();
        self.terminal.enter_raw_mode();
        self.terminal.hide_cursor();
        self.kitty.query();
        self.render_full();
    }

    pub(super) fn show_help(&mut self) {
        let mut text = String::from("Slash commands:");
        for command in builtin_commands() {
            text.push_str(&format!(
                "\n  /{:<14} {}",
                command.name, command.description
            ));
        }
        text.push_str(
            "\n\n\
             Keyboard shortcuts:\n\
             \x20 Enter          Send message (idle) / queue follow-up (running)\n\
             \x20 Shift+Enter    Insert newline\n\
             \x20 Alt+Enter      Insert newline\n\
             \x20 Escape         Abort/cancel running agent or clear editor\n\
             \x20 Esc Esc        Choose a previous turn to go back to\n\
             \x20 Ctrl+C         Clear editor first, abort if empty\n\
             \x20 Ctrl+D         Exit (persists, then lets owned agents settle their subagents)\n\
             \x20 Ctrl+G         Jump to latest conversation output\n\
             \x20 Ctrl+L         Open model selector\n\
             \x20 Ctrl+T         Open tool policy selector\n\
             \x20 Ctrl+O         Toggle tool output expansion\n\
             \x20 Ctrl+Shift+A   Toggle workflow auto-continue\n\
             \x20 Ctrl+Shift+N   Toggle workflow completion nudge\n\
             \x20 Ctrl+Z         Suspend (resume with fg)\n\
             \x20 PageUp/Down    Scroll chat\n\
             \x20 Up/Down        Input history\n\
             \x20 Ctrl+W         Delete previous whitespace-delimited word\n\
             \x20 Alt+D          Delete next Unicode letter/digit word\n\
             \x20 Ctrl+Y         Reinsert the latest line/word deletion\n\
             \x20 Ctrl+Left/Right  Word-movement aliases\n\
             \x20                (Alt/modified arrows depend on terminal support)\n\
             \n\
             \n\
             Mouse / links:\n\
             \x20 Wheel          Scroll chat\n\
             \x20 Drag           Select text\n\
             \x20 Shift+click    Open markdown/OSC 8 link in browser\n\
             \x20                (when mouse capture is on; some terminals\n\
             \x20                use Ctrl/Cmd+click instead)\n",
        );
        self.ac_mut()
            .master_session
            .chat
            .add_entry(ChatEntry::Status { text });
    }

    pub(super) fn show_workflow_status(&mut self) {
        let wf = &self.ac().master_session.workflow_bar;
        let text = if workflow_bar::render_widget(wf, self.terminal.width).is_empty() {
            "Workflow is not active. Ask the agent to select/start a workflow, or launch with `--workflow` to prompt workflow mode immediately.".to_string()
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
        self.ac_mut()
            .master_session
            .chat
            .add_entry(ChatEntry::Status { text });
    }

    pub(super) fn toggle_workflow_auto_continue(&mut self) {
        let next = !self.ac().workflow.auto_continue;
        self.send_command(Command::SetWorkflowAutomation {
            id: Some(self.ac().namespaced_id("workflow-auto")),
            auto_continue: Some(next),
            completion_nudge: None,
        });
    }

    pub(super) fn toggle_workflow_completion_nudge(&mut self) {
        let next = !self.ac().workflow.completion_nudge;
        self.send_command(Command::SetWorkflowAutomation {
            id: Some(self.ac().namespaced_id("workflow-nudge")),
            auto_continue: None,
            completion_nudge: Some(next),
        });
    }

    pub(super) fn send_session_stats(&mut self) {
        self.send_command(Command::GetSessionStats {
            id: Some(self.ac().namespaced_id("stats")),
        });
    }

    /// Request session stats for a quiet footer-only refresh (no chat Status
    /// line). Routed by the "stats-footer" id in the response handler.
    pub(super) fn send_session_stats_footer(&mut self) {
        self.send_command(Command::GetSessionStats {
            id: Some(self.ac().namespaced_id("stats-footer")),
        });
    }

    /// Update the footer's context/cost indicators from a session-stats
    /// payload without emitting a chat entry.
    pub(super) fn update_footer_stats(&mut self, data: &serde_json::Value) {
        let stats = session_payloads::parse_session_stats(data);
        if stats.context_usage.is_some() {
            self.ac_mut().sessions.context_stats_requested = true;
        }
        self.ac_mut()
            .master_session
            .footer
            .apply_session_stats(&stats);
    }

    pub(super) fn show_session_stats(&mut self, data: &serde_json::Value) {
        self.update_footer_stats(data);
        let stats = session_payloads::parse_session_stats(data);
        self.ac_mut()
            .master_session
            .chat
            .add_entry(ChatEntry::Status {
                text: app_session_stats_text::session_stats_text(&stats),
            });
    }


    pub(super) fn open_resume_selector(&mut self, data: &serde_json::Value) {
        self.open_resume_selector_at(
            data,
            &crate::shell::workspace_manifest::default_manifest_path(),
        );
    }

    /// Testable resume selector open with an explicit manifest path (#1465 AC5).
    pub(super) fn open_resume_selector_at(
        &mut self,
        data: &serde_json::Value,
        manifest_path: &std::path::Path,
    ) {
        let mut sessions = session_payloads::parse_resume_sessions(data);
        sessions.sort_by_key(|s| std::cmp::Reverse(s.updated_unix_secs.unwrap_or(0)));
        let empty_hint = if sessions.is_empty() {
            if session_payloads::has_session_entries(data) {
                Some("No resumable CLI sessions found.")
            } else {
                Some("No persisted sessions found.")
            }
        } else {
            None
        };
        self.ac_mut().sessions.resume_items = sessions;
        self.ac_mut().sessions.resume_global = false;
        self.ac_mut().sessions.resume_focus = app_sessions::ResumePickerFocus::Scope;
        let session_items = self.resume_picker_items();
        self.open_resume_selector_with_workspaces(session_items, manifest_path, empty_hint);
    }

    fn resume_picker_items(&self) -> Vec<SelectItem> {
        let global = self.ac().sessions.resume_global;
        self.ac().sessions.resume_items.iter()
            .filter(|session| global || (!session.legacy_unscoped && session.is_local != Some(false)))
            .map(|session| {
                let when = session.updated_unix_secs.map(format_unix_minutes)
                    .unwrap_or_else(|| "unknown time".to_string());
                let location = if session.legacy_unscoped {
                    "legacy unscoped".to_string()
                } else {
                    session.repository_label.as_deref()
                        .zip(session.execution_location.as_deref())
                        .map(|(repo, path)| format!("{repo} · {path}"))
                        .or_else(|| session.execution_location.clone())
                        .unwrap_or_else(|| "scope unavailable".to_string())
                };
                SelectItem {
                    value: format!("session:{}", session.key),
                    label: session.title.clone(),
                    description: Some(format!("{location} · {when} · {} msgs", session.message_count)),
                }
            }).collect()
    }

    pub(super) fn handle_resume_selector_key(&mut self, key: &Key) {
        use app_sessions::ResumePickerFocus;
        if matches!(key, Key::Tab | Key::BackTab) {
            self.ac_mut().sessions.resume_focus = match (self.ac().sessions.resume_focus, key) {
                (ResumePickerFocus::Scope, Key::Tab) => ResumePickerFocus::Query,
                (ResumePickerFocus::Query, Key::Tab) => ResumePickerFocus::Results,
                (ResumePickerFocus::Results, Key::Tab) => ResumePickerFocus::Scope,
                (ResumePickerFocus::Scope, Key::BackTab) => ResumePickerFocus::Results,
                (ResumePickerFocus::Results, Key::BackTab) => ResumePickerFocus::Query,
                (ResumePickerFocus::Query, Key::BackTab) => ResumePickerFocus::Scope,
                _ => unreachable!("allowlisted focus navigation is exhaustive"),
            };
            return;
        }
        if self.ac().sessions.resume_focus == ResumePickerFocus::Scope
            && matches!(key, Key::Enter | Key::Char(' '))
        {
            self.ac_mut().sessions.resume_global = !self.ac().sessions.resume_global;
            let items = self.resume_picker_items();
            if let Some(selector) = self.ac_mut().sessions.resume_selector.as_mut() { selector.sync_items(items); }
            return;
        }
        if let Some(choice) = route_overlay_key(&mut self.ac_mut().sessions.resume_selector, key) {
            let key = choice.strip_prefix("session:").unwrap_or(&choice).to_string();
            let selected = self.ac().sessions.resume_items.iter().find(|item| item.key == key).cloned();
            if selected.as_ref().is_some_and(|item| item.is_local == Some(false) || item.legacy_unscoped) {
                let missing = selected.as_ref().and_then(|item| item.execution_location.as_deref()).is_none();
                let mut actions = Vec::new();
                if missing {
                    actions.push(SelectItem { value: "locate".into(), label: "Locate folder".into(), description: Some("Validate and explicitly reassociate".into()) });
                } else {
                    actions.push(SelectItem { value: "open_original".into(), label: "Open original folder".into(), description: Some("Start a fresh runtime with folder config/tools".into()) });
                }
                actions.push(SelectItem { value: "fork_current".into(), label: "Fork into current folder".into(), description: Some("Transcript only; new identity".into()) });
                actions.push(SelectItem { value: "cancel".into(), label: "Cancel".into(), description: None });
                self.ac_mut().sessions.pending_decision_session = Some(key);
                self.ac_mut().sessions.resume_decision = Some(SelectList::new(actions, 4));
            } else {
                self.apply_resume_selection(&choice);
            }
        }
    }

    pub(super) fn handle_resume_picker_click(&mut self, col: usize, row: usize) {
        let Some((left, top, width, height)) = self.ac().sessions.resume_bounds else { return };
        if !(left..left.saturating_add(width)).contains(&col)
            || !(top..top.saturating_add(height)).contains(&row) { return; }
        if row == top {
            self.ac_mut().sessions.resume_focus = app_sessions::ResumePickerFocus::Scope;
            let global = col >= left.saturating_add(width / 2);
            if self.ac().sessions.resume_global != global {
                self.ac_mut().sessions.resume_global = global;
                let items = self.resume_picker_items();
                if let Some(selector) = self.ac_mut().sessions.resume_selector.as_mut() { selector.sync_items(items); }
            }
            return;
        }
        let index = row.saturating_sub(top.saturating_add(2));
        let count = self.ac().sessions.resume_selector.as_ref().map_or(0, SelectList::len);
        if index < count {
            self.ac_mut().sessions.resume_focus = app_sessions::ResumePickerFocus::Results;
            for _ in 0..index { self.handle_resume_selector_key(&Key::Down); }
            self.handle_resume_selector_key(&Key::Enter);
        }
    }

    pub(super) fn handle_resume_decision_key(&mut self, key: &Key) {
        let choice = route_overlay_key(&mut self.ac_mut().sessions.resume_decision, key);
        let Some(action) = choice else { return };
        let session = self.ac_mut().sessions.pending_decision_session.take().unwrap_or_default();
        if action == "cancel" || session.is_empty() { return; }
        let location = self.ac().sessions.resume_items.iter().find(|item| item.key == session)
            .and_then(|item| item.execution_location.clone());
        self.send_command(Command::ResumeDecision {
            id: Some(self.ac().namespaced_id("resume-decision")), session, action, location,
        });
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
                self.ac_mut()
                    .master_session
                    .chat
                    .add_entry(ChatEntry::Status { text: text.clone() });
                self.notify(&text, NotifyLevel::Error);
                return false;
            }
        };

        let has_displayable_messages = !messages.is_empty();
        self.ac_mut().master_session.chat.clear();
        for entry in Self::resumed_chat_entries(messages) {
            self.ac_mut().master_session.chat.add_entry(entry);
        }
        if !has_displayable_messages {
            self.ac_mut()
                .master_session
                .chat
                .add_entry(ChatEntry::Status {
                    text: empty_status.to_string(),
                });
        }
        has_displayable_messages
    }


    pub(super) fn notify(&mut self, message: &str, level: NotifyLevel) {
        self.notifications.push(Notification::new(message, level));
    }


    /// Diagnostic: append one frame (ANSI-stripped) to the render log.
    fn log_render_frame(&self, path: &str, bottom: &[String]) {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
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

        let active_is_master = self.ac().roster.active_agent_id.is_none();
        let active_spinner_visible = self.ac().spinner.is_some();
        let active_roster_empty = self.ac().roster.tracked.is_empty();
        if active_is_master && active_spinner_visible {
            if let Some(spinner) = &mut self.ac_mut().spinner {
                if active_roster_empty {
                    bottom.push(String::new());
                }
                bottom.extend(spinner.render(width));
            }
        } else if self.active_subagent_running() {
            bottom.push(String::new());
            bottom.push(subagent_activity_line(1, self.ac().roster.frame));
        } else if !self.ac().roster.tracked.is_empty() {
            let roster = &self.ac().roster;
            let active = roster.tracked_active_count();
            if active > 0 {
                bottom.push(subagent_activity_line(active, roster.frame));
            } else {
                bottom.push(subagent_idle_line(roster.tracked.len()));
            }
        }

        bottom.extend(self.autocomplete.render(width));
        bottom.extend(self.workspace.files_autocomplete.render(width));
        self.editor
            .set_show_cursor(!matches!(self.subagents.focus, Focus::Panel));
        bottom.extend(self.editor.render(width));
        bottom.extend(self.notifications.render(width));
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

        let panel_visible = self.subagent_panel_visible();
        let (panel_width, _divider_width, width) = self.frame_split();

        let now = tokio::time::Instant::now();

        let mut lines = Vec::new();

        let bottom = self.compose_bottom(width);
        let bottom_height = bottom.len();

        lines.push(String::new());

        let main_box_width = width;
        let main_pane_workflow = self.render_main_pane_workflow(width, main_box_width, now);
        lines.extend(main_pane_workflow);

        let top_chrome_height = lines.len();
        let unpadded_chat_height = height.saturating_sub(bottom_height + top_chrome_height);
        let streaming_vertical_padding = usize::from(unpadded_chat_height > 1);
        let chat_height = unpadded_chat_height.saturating_sub(streaming_vertical_padding);
        let (mut chat_lines, show_latest_tip) = match self.render_environment_body(width) {
            Some(body) => (Self::clamp_environment_body(body, chat_height), false),
            None => {
                let chat = self.active_chat_mut();
                chat.set_viewport_height(chat_height);
                let mut lines = chat.render(width);
                if lines.len() > chat_height {
                    lines = lines[lines.len() - chat_height..].to_vec();
                }
                (lines, chat.is_scrolled_from_latest())
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
        lines.extend(bottom);
        if lines.len() > height {
            let start = lines.len() - height;
            lines = lines[start..].to_vec();
        }
        while lines.len() < height {
            lines.push(String::new());
        }
        if show_latest_tip && width > 0 {
            let tip = "Ctrl + G - Jump Back to Latest Message";
            let overlay_width = crate::components::utils::visible_width(tip).min(width);
            let overlay =
                crate::components::utils::truncate_to_width(&theme::red(tip), overlay_width, None);
            let row = available
                .saturating_sub(1)
                .min(lines.len().saturating_sub(1));
            let col = width.saturating_sub(overlay_width) / 2;
            crate::components::overlay::splice_frame_line(
                &mut lines,
                row,
                &overlay,
                col,
                overlay_width,
                width,
            );
        }
        if let Some(selector) = &mut self.ac_mut().sessions.resume_selector {
            let (selector_lines, overlay_width) =
                build_resume_selector_overlay(selector, width, height);
            let overlay_height = selector_lines.len().min(height.saturating_sub(4));
            self.ac_mut().sessions.resume_bounds = Some((
                width.saturating_sub(overlay_width) / 2,
                height.saturating_sub(overlay_height) / 2,
                overlay_width,
                overlay_height,
            ));
            Self::composite_centered(&mut lines, &selector_lines, overlay_width, width, height);
        }
        if let Some(selector) = &mut self.ac_mut().sessions.resume_decision {
            let (selector_lines, overlay_width) = build_select_list_overlay(
                "Resume from another folder", "Enter choose · Esc cancel", selector, width, height,
            );
            Self::composite_centered(&mut lines, &selector_lines, overlay_width, width, height);
        }
        if let Some(selector) = &mut self.ac_mut().rewind.selector {
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

        if self.selection.is_some() {
            self.last_rendered_lines = lines.clone();
        } else {
            self.last_rendered_lines.clear();
        }
        lines
    }

    /// Splice a centered overlay into `lines`, in place. Centers `overlay_lines` (clamped to leave a 4-row margin) and splices
    /// each row through the ANSI-aware splice helper so escape codes from the
    /// underlying frame can't bleed into or out of the overlay. Shared
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
            if i < overlay_lines.len() {
                crate::components::overlay::splice_frame_line(
                    lines,
                    row,
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
        if let Some(path) = self.render_log_path.as_deref() {
            self.log_render_frame(path, &lines);
        }

        let (panel_width, divider_width, _body_width) = self.frame_split();
        let body_start_col = panel_width
            .saturating_add(divider_width)
            .min(u16::MAX as usize) as u16;
        apply_selection_highlight(&self.selection, &mut lines, body_start_col);

        if let Err(e) = self.renderer.render(&lines, self.terminal.width) {
            self.handle_render_failure(&e);
        }
    }

    pub(super) fn handle_render_failure(&mut self, error: &std::io::Error) {
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

    /// Start a fresh single `/new` session, preserving the old session for `/resume`.
    pub(super) fn reset_workspace(&mut self) -> Vec<crate::shell::child_watch::ChildWatch> {
        self.persist_default_durability();

        let mut master = self
            .tabs
            .remove(&crate::shell::connection::TabId::MASTER)
            .expect("workspace reset requires a master tab");
        let mut watches = Vec::new();
        for (_, mut state) in self.tabs.drain() {
            state.transport.abort_feed();
            watches.extend(state.child_exit_watch.take());
        }
        master.name = None;
        master.session_key = None;
        master.pending_session_resume = None;
        master.roster = crate::agents::view::ConnectionRoster::new();
        self.tabs
            .insert(crate::shell::connection::TabId::MASTER, master);
        self.active_tab = crate::shell::connection::TabId::MASTER;
        self.routing_tab_override = None;
        self.editor.set_text("");
        self.subagents = crate::agents::view::SubagentUi::new();
        self.workspace_id = crate::shell::workspace_manifest::generate_workspace_id();
        self.workspace_label = crate::shell::workspace_manifest::generate_workspace_label();
        self.reset_session("New session started");
        self.persist_default_durability();
        watches
    }

    /// Reset the conversation — clears agent history, chat UI, and context display.
    pub(super) fn reset_session(&mut self, message: &str) {
        self.ac_mut().disconnect_diag_pending = false;
        // identical to pre-seam master; command acks are phase-2 scope.
        let was_connected = self.ac().agent_connected;
        let agent_reset = self.send_new_session();
        // Clearing the local conversation intentionally abandons the old
        // session view even when the transport cannot accept new_session.
        self.ac_mut()
            .reset_coordinator_clock(tokio::time::Instant::now());
        self.ac_mut().master_session.chat.clear();
        // The clear wiped any persistent refusal Status line; re-arm the
        // once-per-episode latch so the next refusal (send_state_resync
        // below, on a dead connection) re-raises the toast and re-writes
        // the line into the fresh transcript (#1470 r6).
        if !self.ac().agent_connected {
            self.ac_mut().disconnect_refusal_notified = false;
        }
        // Invalidate in-flight ref recovery so a late get_message from the OLD
        // transcript can't splice into the cleared /clear-or-/new session (#1060 r4).
        self.clear_message_recovery();
        self.ac_mut().master_session.footer.set_context(None, 0);
        self.ac_mut().sessions.context_stats_requested = false;
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
}
