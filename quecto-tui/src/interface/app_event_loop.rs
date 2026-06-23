use super::app_selection::{SelectionAnchor, TextSelection};
use super::*;

impl App {
    pub async fn run(&mut self) -> i32 {
        self.terminal.enter_raw_mode();
        self.terminal.hide_cursor();

        // Query Kitty keyboard protocol support.
        self.kitty.query();

        // Query initial state from agent.
        let _ = self
            .client
            .send(&Command::GetState {
                id: Some("init".into()),
            })
            .await;
        // Query initial subagent state (#525).
        let _ = self
            .client
            .send(&Command::GetSubagents {
                id: Some("init-subagents".into()),
            })
            .await;

        // Set up SIGWINCH handler.
        let mut resize_rx = crate::infrastructure::signals::sigwinch_stream().await;

        // Set up stdin reader (async, byte-level).
        let (stdin_tx, mut stdin_rx) = mpsc::channel::<Vec<u8>>(64);
        tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut stdin = tokio::io::stdin();
            let mut buf = [0u8; 256];
            loop {
                match stdin.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        if stdin_tx.send(buf[..n].to_vec()).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        // Kitty protocol fallback timer.
        let kitty_deadline = tokio::time::Instant::now() + Duration::from_millis(150);
        let mut kitty_fallback_done = false;

        // Initial render.
        self.render();

        // Spinner tick timer.
        let mut spinner_interval = tokio::time::interval(SPINNER_TICK);
        spinner_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        // Git branch footer refresh timer.
        let mut git_branch_interval = tokio::time::interval(app_git::GIT_BRANCH_POLL_INTERVAL);
        git_branch_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let (git_branch_tx, mut git_branch_rx) = mpsc::channel::<Option<String>>(1);
        let mut git_branch_refresh_in_flight = false;

        // Timeout for incomplete escape sequences (matches Quecto TUI's 10ms).
        let escape_timeout = Duration::from_millis(10);

        // Main event loop.
        loop {
            if self.should_exit {
                break;
            }

            tokio::select! {
                // Stdin input.
                Some(bytes) = stdin_rx.recv() => {
                    // Check for Kitty protocol response before buffering.
                    if !self.kitty.active && !kitty_fallback_done {
                        if let Some(_flags) = KittyProtocol::parse_response(&bytes) {
                            self.kitty.enable();
                            kitty_fallback_done = true;
                            continue;
                        }
                    }
                    // Feed bytes into the proper StdinBuffer. Kitty release events are
                    // filtered per decoded sequence in `process_key_sequence`; filtering
                    // the raw read can drop a legitimate press when a press+release arrive
                    // in the same read.
                    self.stdin_buffer.feed(&bytes);

                    // Drain complete sequences immediately.
                    let complete = self.stdin_buffer.drain_complete();
                    for seq in &complete {
                        self.process_key_sequence(seq);
                        if self.should_exit { break; }
                    }

                    // If there are still pending bytes (incomplete escape),
                    // retry up to MAX_ESCAPE_RETRIES times waiting for more data.
                    // This handles 3+ fragment CSI splits on slow SSH/serial (#466).
                    if self.stdin_buffer.has_pending() && !self.should_exit {
                        let mut retries = 0;
                        while self.stdin_buffer.has_pending()
                            && retries < MAX_ESCAPE_RETRIES
                            && !self.should_exit
                        {
                            retries += 1;
                            match tokio::time::timeout(escape_timeout, stdin_rx.recv()).await {
                                Ok(Some(more)) => {
                                    self.stdin_buffer.feed(&more);
                                    let seqs = self.stdin_buffer.drain_complete();
                                    for seq in &seqs {
                                        self.process_key_sequence(seq);
                                        if self.should_exit { break; }
                                    }
                                }
                                Ok(None) => break, // Channel closed (stdin EOF).
                                Err(_) => break,   // Timeout — no more data coming.
                            }
                        }
                        // Force drain anything still pending after retries
                        // exhausted (bare Escape, etc.).
                        if !self.should_exit {
                            let forced = self.stdin_buffer.drain_all();
                            for seq in &forced {
                                self.process_key_sequence(seq);
                                if self.should_exit { break; }
                            }
                        }
                    }

                    self.render();
                }
                // Agent events.
                event = self.client.recv(), if self.agent_connected => {
                    match event {
                        Some(ev) => {
                            self.handle_event(ev);
                            self.render();
                        }
                        None => {
                            // Agent disconnected — stop polling.
                            self.agent_connected = false;
                            self.agent_state.reset();
                            self.spinner = None;
                            self.chat.finalize_assistant();
                            self.notify("Agent disconnected", NotifyLevel::Error);
                            self.render();
                        }
                    }
                }
                // Terminal resize.
                Some(()) = resize_rx.recv() => {
                    self.terminal.refresh_size();
                    self.render_full();
                }
                // Spinner tick.
                _ = spinner_interval.tick() => {
                    if self.handle_periodic_tick(&mut kitty_fallback_done, kitty_deadline) {
                        self.render();
                    }
                }
                Some(files) = self.files_load_rx.recv() => {
                    self.files_load_in_flight = false;
                    self.files_autocomplete.complete_load(files);
                    self.render();
                }
                Some(message) = self.command_error_rx.recv() => {
                    self.notify(&message, NotifyLevel::Error);
                    self.render();
                }
                Some(branch) = git_branch_rx.recv() => {
                    git_branch_refresh_in_flight = false;
                    if self.apply_git_branch(branch) {
                        self.render();
                    }
                }
                // Git branch footer refresh tick.
                _ = git_branch_interval.tick() => {
                    // Git branch may change while the TUI is running (checkout/switch
                    // from another shell or from commands the agent runs). Refresh it
                    // periodically so the footer does not stay pinned to the startup
                    // branch. The filesystem read runs off the UI loop so slow or
                    // unusual repositories cannot block input, rendering, or agent events.
                    self.start_git_branch_refresh(&git_branch_tx, &mut git_branch_refresh_in_flight);
                }
            }
        }

        // Cleanup.
        self.kitty.cleanup();
        self.terminal.show_cursor();
        self.terminal.exit_raw_mode();
        self.terminal.write_str("\r\n");
        0
    }

    fn handle_periodic_tick(
        &mut self,
        kitty_fallback_done: &mut bool,
        kitty_deadline: tokio::time::Instant,
    ) -> bool {
        let spinner_changed = self.spinner.as_mut().is_some_and(|spinner| spinner.tick());
        let notifications_changed = self.notifications.gc();
        let subagents_changed = self.gc_exited_subagents() || self.tick_subagent_animation();
        if !*kitty_fallback_done && tokio::time::Instant::now() >= kitty_deadline {
            if !self.kitty.active {
                self.kitty.enable_modify_other_keys();
            }
            *kitty_fallback_done = true;
        }
        spinner_changed || notifications_changed || subagents_changed
    }

    // ── Input handling ────────────────────────────────────────────────

    /// Process a single complete key sequence from the StdinBuffer.
    pub(super) fn process_key_sequence(&mut self, seq: &[u8]) {
        // Kitty key release events can arrive in the same stdin read as a key
        // press. Filtering only the raw read can miss them, causing one arrow
        // key press to be handled twice. Drop releases per decoded sequence.
        if self.kitty.active && crate::interface::kitty::is_key_release(seq) {
            return;
        }
        if let Some((key, _)) = keys::parse_key(seq) {
            self.handle_key(key);
        }
    }

    pub(super) fn handle_key(&mut self, key: Key) {
        if !matches!(key, Key::Escape) {
            self.last_idle_escape = None;
        }

        // Unconditional exit — Ctrl+D must work regardless of overlays,
        // autocomplete state, or agent activity (#478).
        if matches!(key, Key::Ctrl('d')) {
            if self.agent_state.is_running() {
                self.handle_abort();
            }
            self.should_exit = true;
            return;
        }

        // If the resume selector is active, route input to it.
        if self.resume_selector.is_some() {
            self.last_idle_escape = None;
            self.handle_resume_selector_key(&key);
            return;
        }

        // If the rewind selector is active, route input to it.
        if self.rewind_selector.is_some() {
            self.last_idle_escape = None;
            self.handle_rewind_selector_key(&key);
            return;
        }

        // If the model selector is active, route input to it.
        if self.model_selector.is_some() {
            self.last_idle_escape = None;
            self.handle_model_selector_key(&key);
            return;
        }

        // If an overlay is active, route input there.
        if self.overlay_stack.has_visible() {
            self.last_idle_escape = None;
            if let Some(entry) = self.overlay_stack.topmost_entry_mut() {
                entry.component.handle_input(&key);
            }
            // Check if overlay wants to close (Escape).
            if matches!(key, Key::Escape) {
                self.overlay_stack.pop();
            }
            return;
        }

        // If autocomplete is active, route navigation keys there.
        if self.autocomplete.is_active() {
            match &key {
                Key::Up | Key::Down | Key::Tab | Key::Escape => {
                    if matches!(key, Key::Escape) {
                        self.last_idle_escape = None;
                    }
                    self.autocomplete.handle_input(&key);
                    // Check if a suggestion was selected.
                    match self.autocomplete.take_result() {
                        AutocompleteResult::Selected(value) => {
                            self.editor.set_text(&value);
                        }
                        AutocompleteResult::Dismissed => {}
                        AutocompleteResult::Pending => {}
                    }
                    return;
                }
                Key::Enter => {
                    // Accept the highlighted suggestion AND submit it.
                    self.autocomplete.handle_input(&Key::Tab);
                    if let AutocompleteResult::Selected(value) = self.autocomplete.take_result() {
                        self.editor.set_text(&value);
                        self.editor.add_to_history(value.trim());
                        self.autocomplete.dismiss();
                        self.handle_submit(&value);
                        self.editor.set_text("");
                    }
                    return;
                }
                _ => {
                    // Other keys go to editor, then update autocomplete.
                }
            }
        }

        // If the @files autocomplete is active, route navigation keys there.
        // Unlike slash commands, Enter ACCEPTS the mention but does NOT submit —
        // the `@path` is part of a longer message.
        if self.files_autocomplete.is_active() {
            match &key {
                Key::Up | Key::Down | Key::Tab | Key::Escape => {
                    self.files_autocomplete.handle_input(&key);
                    if let AutocompleteResult::Selected(path) =
                        self.files_autocomplete.take_result()
                    {
                        self.accept_file_mention(&path);
                    }
                    return;
                }
                Key::Enter => {
                    self.files_autocomplete.handle_input(&Key::Tab);
                    if let AutocompleteResult::Selected(path) =
                        self.files_autocomplete.take_result()
                    {
                        self.accept_file_mention(&path);
                    }
                    return;
                }
                _ => {}
            }
        }

        // Global key handlers.
        // Note: Ctrl+D is handled at the top of handle_key (unconditional exit).
        match &key {
            Key::Ctrl('c') => {
                match ctrl_c_action(self.agent_state.is_running(), self.editor.text().is_empty()) {
                    CtrlCAction::ClearEditor => {
                        self.editor.set_text("");
                        self.autocomplete.dismiss();
                    }
                    CtrlCAction::AbortAgent => {
                        self.handle_abort();
                    }
                    CtrlCAction::Noop => {}
                }
                return;
            }
            Key::Escape => {
                if self.agent_state.is_running() {
                    self.last_idle_escape = None;
                    self.handle_abort();
                } else if !self.editor.text().is_empty() {
                    self.last_idle_escape = None;
                    self.editor.set_text("");
                    self.autocomplete.dismiss();
                } else {
                    self.handle_idle_escape_for_rewind();
                }
                return;
            }
            Key::Ctrl('z') => {
                // Suspend (Ctrl+Z).
                self.kitty.cleanup();
                self.terminal.show_cursor();
                crate::infrastructure::signals::suspend();
                // Resumed — re-enter raw mode.
                self.terminal.enter_raw_mode();
                self.terminal.hide_cursor();
                self.kitty.query();
                self.render_full();
                return;
            }
            Key::Ctrl('l') => {
                // Open model selector overlay.
                self.open_model_selector();
                return;
            }
            Key::Ctrl('o') => {
                // Toggle tool output expansion.
                self.chat.toggle_tool_expand();
                let state = if self.chat.tool_expanded {
                    "expanded"
                } else {
                    "collapsed"
                };
                self.notify(&format!("Tool output {}", state), NotifyLevel::Info);
                return;
            }
            Key::CtrlShift('a') => {
                self.toggle_workflow_auto_continue();
                return;
            }
            Key::CtrlShift('n') => {
                self.toggle_workflow_completion_nudge();
                return;
            }
            Key::MousePress(col, row) => {
                self.selection = Some(TextSelection {
                    start: SelectionAnchor {
                        col: *col,
                        row: *row,
                    },
                    end: SelectionAnchor {
                        col: *col,
                        row: *row,
                    },
                });
                return;
            }
            Key::MouseDrag(col, row) => {
                if let Some(sel) = &mut self.selection {
                    sel.end = SelectionAnchor {
                        col: *col,
                        row: *row,
                    };
                }
                return;
            }
            Key::MouseRelease(col, row) => {
                if let Some(sel) = self.selection.take() {
                    let end = SelectionAnchor {
                        col: *col,
                        row: *row,
                    };
                    // Only copy if there's an actual selection (not just a click).
                    if sel.start.row != end.row || sel.start.col != end.col {
                        let text = self.extract_selection(&sel.start, &end);
                        if !text.is_empty() {
                            copy_to_clipboard(&text);
                            let chars = text.chars().count();
                            self.notify(
                                &format!("Copied {} chars to clipboard", chars),
                                NotifyLevel::Success,
                            );
                        }
                    }
                }
                return;
            }
            Key::ScrollUp => {
                self.chat.scroll_up(MOUSE_SCROLL_LINES);
                return;
            }
            Key::ScrollDown => {
                self.chat.scroll_down(MOUSE_SCROLL_LINES);
                return;
            }
            Key::PageUp => {
                self.chat.scroll_up(10);
                return;
            }
            Key::PageDown => {
                self.chat.scroll_down(10);
                return;
            }
            _ => {}
        }

        // Forward to editor.
        self.editor.handle_input(&key);

        // Update autocomplete after every editor change.
        self.autocomplete.update(&self.editor.text());

        // Update the @files popup from the cursor's line (slash takes priority).
        if self.autocomplete.is_active() {
            self.files_autocomplete.dismiss();
        } else {
            let line = self.editor.current_line().to_string();
            let col = self.editor.cursor_col();
            self.files_autocomplete.update(&line, col);
            self.maybe_start_files_load();
        }

        // Check if editor submitted.
        if let Some(text) = self.editor.take_submit() {
            self.autocomplete.dismiss();
            self.handle_submit(&text);
        }
    }

    /// Replace the active `@token` in the editor with the selected file path.
    fn accept_file_mention(&mut self, path: &str) {
        if let Some(start) = self.files_autocomplete.token_start() {
            self.editor
                .replace_before_cursor(start, &format!("@{path} "));
        }
        self.files_autocomplete.dismiss();
    }

    pub(super) fn handle_submit(&mut self, text: &str) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }

        // Slash commands.
        if trimmed.starts_with('/') {
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
                    let session = trimmed.strip_prefix("/resume").unwrap().trim();
                    self.send_resume_session(session);
                    return;
                }
                _ if trimmed.starts_with("/model") => {
                    let model_name = trimmed.strip_prefix("/model").unwrap().trim();
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

        // Add user message to chat.
        self.chat.add_entry(ChatEntry::User {
            text: text.to_string(),
        });

        // Send to agent.
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
        // Send abort to agent.
        self.send_command(Command::Abort { id: None });

        // Call abort on the state machine — does NOT set running to false.
        // The AgentEnd event will arrive and be matched against the current
        // generation, preventing stale events from corrupting state (#502).
        self.agent_state.abort();
        self.footer.set_streaming(false);

        // Stop spinner (visual feedback that abort was acknowledged).
        self.spinner = None;

        // Finalize any streaming assistant message.
        self.chat.finalize_assistant();

        // Show abort status.
        self.chat.add_entry(ChatEntry::Status {
            text: "Operation aborted".to_string(),
        });
    }
}
