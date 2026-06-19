use super::*;

impl App {
    pub(super) fn update_subagent_bar(
        &mut self,
        subagents: Vec<crate::infrastructure::client::SubagentInfoEvent>,
    ) {
        // Merge server data with existing local state to preserve exited_at
        // timestamps. New entries are inserted; entries absent from the
        // server push are removed unless they have an active grace period.
        let mut new_map = std::collections::BTreeMap::new();
        for s in subagents {
            let id = sanitize_agent_id(&s.agent_id);
            if let Some(mut existing) = self.subagent_local.remove(&id) {
                existing.update_info(s);
                new_map.insert(id, existing);
            } else {
                new_map.insert(id, TrackedSubagent::new(s));
            }
        }
        // Preserve locally-tracked exited entries whose grace period
        // hasn't elapsed yet (server may stop reporting them immediately).
        let now = tokio::time::Instant::now();
        for (id, entry) in std::mem::take(&mut self.subagent_local) {
            if let Some(exited_at) = entry.exited_at {
                if now.saturating_duration_since(exited_at) < EXITED_SUBAGENT_GRACE {
                    new_map.entry(id).or_insert(entry);
                }
            }
        }
        self.subagent_local = new_map;
        self.rebuild_subagent_bar();
    }

    /// Rebuild the widget from local state.
    pub(super) fn rebuild_subagent_bar(&mut self) {
        if self.subagent_local.is_empty() {
            self.widgets_above.clear("subagents");
        } else {
            let now = tokio::time::Instant::now();
            let rows: Vec<SubagentRow> = self
                .subagent_local
                .values()
                .map(|t| SubagentRow::new(t.info.clone(), t.elapsed_secs(now)))
                .collect();
            let mut bar = SubagentBar::new();
            bar.update(rows, self.subagent_frame);
            bar.set_awaited(self.awaited_agent_id.clone());
            self.widgets_above.set("subagents", Box::new(bar));
        }
    }

    /// Advance the subagent spinner animation. Returns `true` if a re-render is
    /// needed (i.e. at least one agent is active). Driven by the spinner tick so
    /// running agents animate and their elapsed-time clocks stay current.
    pub(super) fn tick_subagent_animation(&mut self) -> bool {
        let any_active = self
            .subagent_local
            .values()
            .any(|t| subagent_status_is_active(&t.info.status));
        if !any_active {
            return false;
        }
        self.subagent_frame = self.subagent_frame.wrapping_add(1);
        self.rebuild_subagent_bar();
        true
    }

    /// GC exited subagent bars whose grace period has elapsed (#540).
    /// Returns `true` if the bar was modified.
    pub(super) fn gc_exited_subagents(&mut self) -> bool {
        if self.subagent_local.is_empty() {
            return false;
        }
        let removed = gc_exited_subagents(
            &mut self.subagent_local,
            tokio::time::Instant::now(),
            EXITED_SUBAGENT_GRACE,
        );
        if removed {
            self.rebuild_subagent_bar();
        }
        removed
    }

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
        self.chat.add_entry(ChatEntry::Status {
            text: [
                "Keyboard shortcuts:",
                "  Enter          Send message",
                "  Shift+Enter    Insert newline",
                "  Alt+Enter      Insert newline",
                "  Escape         Abort agent / clear editor",
                "  Ctrl+C         Clear editor first, abort if empty",
                "  Ctrl+D         Exit",
                "  Ctrl+L         Open model selector",
                "  Ctrl+O         Toggle tool output expansion",
                "  Ctrl+Shift+A   Toggle workflow auto-continue",
                "  Ctrl+Shift+N   Toggle workflow completion nudge",
                "  Ctrl+Z         Suspend (resume with fg)",
                "  PageUp/Down    Scroll chat",
                "  Up/Down        Input history",
                "",
                "Slash commands:",
                "  /model         Open model selector",
                "  /model <name>  Switch to model directly",
                "  /clear         Clear conversation",
                "  /new           New session",
                "  /session       Show session info",
                "  /workflow      Show workflow status",
                "  /resume        Pick a persisted session to resume",
                "  /resume <name> Resume a persisted session directly",
                "  /workflow-auto Toggle workflow auto-continue",
                "  /workflow-nudge Toggle workflow completion nudge",
                "  /help,/hotkeys This help",
                "  /quit,/exit    Exit",
            ]
            .join("\n"),
        });
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
        let key = data
            .get("sessionKey")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let msgs = data
            .get("totalMessages")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let input = data
            .get("tokens")
            .and_then(|t| t.get("input"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let output = data
            .get("tokens")
            .and_then(|t| t.get("output"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let cost = data.get("cost").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let max_context_tokens = data
            .get("maxContextTokens")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);
        if let Some(window) = max_context_tokens {
            self.footer.update_context_usage(input, window);
            self.context_stats_requested = true;
        }

        self.chat.add_entry(ChatEntry::Status {
            text: format!(
                "Session: {} | Messages: {} | Tokens: ↑{} ↓{} | Cost: ${:.4}",
                key, msgs, input, output, cost
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
        let sessions = data
            .get("sessions")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if sessions.is_empty() {
            self.chat.add_entry(ChatEntry::Status {
                text: "No persisted sessions found.".to_string(),
            });
            return;
        }
        let items = sessions
            .into_iter()
            .filter_map(|session| {
                let name = session.get("name").and_then(|v| v.as_str())?;
                let count = session
                    .get("messageCount")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                Some(SelectItem {
                    value: name.to_string(),
                    label: name.to_string(),
                    description: Some(format!("{count} messages")),
                })
            })
            .collect::<Vec<_>>();
        if items.is_empty() {
            self.chat.add_entry(ChatEntry::Status {
                text: "No resumable CLI sessions found.".to_string(),
            });
            return;
        }
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
        let messages = data
            .get("messages")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        self.chat.clear();
        for message in messages {
            let role = message.get("role").and_then(|v| v.as_str()).unwrap_or("");
            let content = message
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            match role {
                "user" => self.chat.add_entry(ChatEntry::User { text: content }),
                "assistant" if !content.is_empty() => self.chat.add_entry(ChatEntry::Assistant {
                    text: content,
                    streaming: false,
                }),
                _ => {}
            }
        }
        self.chat.add_entry(ChatEntry::Status {
            text: "Session resumed".to_string(),
        });
    }

    // ── Model selector ──────────────────────────────────────────────

    pub(super) fn open_model_selector(&mut self) {
        let selector = ModelSelector::new(self.current_model.as_deref());
        self.model_selector = Some(selector);
    }

    pub(super) fn handle_model_selector_key(&mut self, key: &Key) {
        if let Some(selector) = &mut self.model_selector {
            selector.handle_input(key);

            match selector.take_result() {
                ModelSelectorResult::Selected(model) => {
                    self.model_selector = None;
                    self.send_set_model(&model);
                }
                ModelSelectorResult::Cancelled => {
                    self.model_selector = None;
                }
                ModelSelectorResult::Pending => {}
            }
        }
    }

    // ── Notifications ─────────────────────────────────────────────────

    pub(super) fn notify(&mut self, message: &str, level: NotifyLevel) {
        self.notifications.push(Notification::new(message, level));
    }

    // ── Rendering ─────────────────────────────────────────────────────

    pub(super) fn render(&mut self) {
        let width = self.terminal.width;
        let height = self.terminal.height;

        let mut lines = Vec::new();
        let mut workflow_bar_state = self.workflow_bar.clone();
        workflow_bar_state.workflow_auto_continue = self.workflow_auto_continue;
        workflow_bar_state.workflow_completion_nudge = self.workflow_completion_nudge;

        // ── Render bottom section first to know its height ──────────
        let mut bottom = Vec::new();

        // Widgets above editor (subagent bars stay on top, visible).
        bottom.extend(self.widgets_above.render(width));

        // Quecto-style workflow widget above the editor.
        let workflow_widget_lines = workflow_bar::render_widget(&workflow_bar_state, width);
        bottom.extend(workflow_widget_lines);

        // Spinner sits between widgets_above and autocomplete (#534). It renders
        // a stable single line for the whole processing turn; per-agent await
        // status is shown on the sub-agent rows. (Suppressing it during await
        // made it flip on/off between consecutive awaits — a sub-second flash.)
        if let Some(spinner) = &mut self.spinner {
            bottom.extend(spinner.render(width));
        }

        // Autocomplete dropdown.
        bottom.extend(self.autocomplete.render(width));

        // Editor.
        bottom.extend(self.editor.render(width));

        // Widgets below editor.
        bottom.extend(self.widgets_below.render(width));

        // Notifications.
        bottom.extend(self.notifications.render(width));

        // Footer.
        bottom.extend(self.footer.render(width));

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
        self.chat.set_viewport_height(chat_height);
        let mut chat_lines = self.chat.render(width);

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

        // Composite overlays on top.
        if self.overlay_stack.has_visible() {
            self.overlay_stack.composite(&mut lines, width, height);
        }

        // Composite resume selector overlay if active.
        // Uses ANSI-aware splice_line to avoid escape code bleeding.
        if let Some(selector) = &mut self.resume_selector {
            let (selector_lines, overlay_width) =
                build_resume_selector_overlay(selector, width, height);
            let overlay_height = selector_lines.len().min(height.saturating_sub(4));
            let start_row = height.saturating_sub(overlay_height) / 2;
            let start_col = width.saturating_sub(overlay_width) / 2;
            for i in 0..overlay_height {
                let row = start_row + i;
                if row < lines.len() && i < selector_lines.len() {
                    lines[row] = crate::interface::overlay::splice_line(
                        &lines[row],
                        &selector_lines[i],
                        start_col,
                        overlay_width,
                        width,
                    );
                }
            }
        }

        // Composite model selector overlay if active.
        // Uses ANSI-aware splice_line to avoid escape code bleeding.
        if let Some(selector) = &mut self.model_selector {
            let overlay_width = width.saturating_sub(4).min(60);
            let selector_lines = selector.render(overlay_width);
            let overlay_height = selector_lines.len().min(height.saturating_sub(4));

            // Center the overlay.
            let start_row = height.saturating_sub(overlay_height) / 2;
            let start_col = width.saturating_sub(overlay_width) / 2;

            for i in 0..overlay_height {
                let row = start_row + i;
                if row < lines.len() && i < selector_lines.len() {
                    lines[row] = crate::interface::overlay::splice_line(
                        &lines[row],
                        &selector_lines[i],
                        start_col,
                        overlay_width,
                        width,
                    );
                }
            }
        }

        // Enforce width on every line.
        for line in &mut lines {
            if crate::interface::utils::visible_width(line) > width {
                *line = crate::interface::utils::truncate_to_width(line, width, None);
            }
        }

        // Store rendered lines for text selection extraction (#528).
        // Must happen BEFORE highlight injection to avoid leaking
        // reverse-video escapes into the extraction buffer (#546 review).
        self.last_rendered_lines = lines.clone();

        // Apply mouse selection highlight (#546).
        // Applied to a separate copy so extraction buffer stays clean.
        apply_selection_highlight(&self.selection, &mut lines);

        // Write to terminal.
        let mut buf = String::new();
        buf.push_str("\x1b[?2026h");
        buf.push_str("\x1b[H");

        for (i, line) in lines.iter().enumerate() {
            if i > 0 {
                buf.push_str("\r\n");
            }
            buf.push_str("\x1b[2K");
            buf.push_str(line);
        }

        let rendered = lines.len();
        if rendered < height {
            for _ in rendered..height {
                buf.push_str("\r\n\x1b[2K");
            }
        }

        buf.push_str("\x1b[?2026l");

        let _ = std::io::stdout().write_all(buf.as_bytes());
        let _ = std::io::stdout().flush();
    }

    pub(super) fn render_full(&mut self) {
        self.terminal.clear_screen();
        self.render();
    }

    /// Reset the conversation — clears agent history, chat UI, and context display.
    pub(super) fn reset_session(&mut self, message: &str) {
        self.send_clear_history();
        self.chat.clear();
        self.footer.set_context(None, 0);
        self.context_stats_requested = false;
        self.notify(message, NotifyLevel::Success);
    }

    pub(super) fn send_clear_history(&mut self) {
        self.send_command(Command::ClearHistory { id: None });
    }

    // ── Command sending ───────────────────────────────────────────────

    pub(super) fn send_command(&mut self, cmd: Command) {
        let mut sender = self.client.clone_sender();
        tokio::spawn(async move {
            let _ = sender.send(&cmd).await;
        });
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
