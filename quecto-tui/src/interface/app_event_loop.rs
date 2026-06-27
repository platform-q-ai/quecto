use super::app_selection::{SelectionAnchor, TextSelection};
use super::*;

impl App {
    pub async fn run(&mut self) -> i32 {
        self.terminal.enter_raw_mode();
        self.terminal.hide_cursor();

        // Query Kitty keyboard protocol support.
        self.kitty.query();

        // Query initial state from agent through the shared command path so
        // startup send failures surface in the UI like user-initiated sends.
        self.send_command(Command::GetState {
            id: Some("init".into()),
        });
        // Query initial subagent state (#525).
        self.send_command(Command::GetSubagents {
            id: Some("init-subagents".into()),
        });

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

        // @files autocomplete workspace enumeration. Loading can shell out to git
        // or walk the filesystem, so keep it off the input/render loop.
        let (files_autocomplete_tx, mut files_autocomplete_rx) = mpsc::channel::<Vec<String>>(1);
        let mut files_autocomplete_load_in_flight = false;

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
                    if !self
                        .process_stdin_bytes(bytes, &mut stdin_rx, escape_timeout, &mut kitty_fallback_done)
                        .await
                    {
                        continue;
                    }

                    if self.files_autocomplete.take_load_request() {
                        self.start_files_autocomplete_load(
                            &files_autocomplete_tx,
                            &mut files_autocomplete_load_in_flight,
                        );
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
                            self.master_session.running = false;
                            self.spinner = None;
                            self.master_session.chat.finalize_assistant();
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
                    let mut needs_render = false;
                    if let Some(spinner) = &mut self.spinner {
                        if spinner.tick() {
                            needs_render = true;
                        }
                    }
                    // GC expired notifications.
                    if self.notifications.gc() {
                        needs_render = true;
                    }
                    // GC exited subagent bars (#540).
                    if self.gc_exited_subagents() {
                        needs_render = true;
                    }
                    // Animate the subagent spinner / advance elapsed-time clocks.
                    if self.tick_subagent_animation() {
                        needs_render = true;
                    }
                    // Kitty fallback — enable modifyOtherKeys if no response.
                    if !kitty_fallback_done && tokio::time::Instant::now() >= kitty_deadline {
                        if !self.kitty.active {
                            self.kitty.enable_modify_other_keys();
                        }
                        kitty_fallback_done = true;
                    }
                    if needs_render {
                        self.render();
                    }
                }
                Some(branch) = git_branch_rx.recv() => {
                    git_branch_refresh_in_flight = false;
                    if self.apply_git_branch(branch) {
                        self.render();
                    }
                }
                Some(failure) = self.command_send_failure_rx.recv() => {
                    self.handle_command_send_failure(failure);
                    self.render();
                }
                // Events fanned in from the active sub-agent's direct
                // connect-on-select connection (#800).
                Some((agent_id, ev)) = self.subagent_event_rx.recv() => {
                    self.route_subagent_event(&agent_id, ev);
                    self.render();
                }
                Some(files) = files_autocomplete_rx.recv() => {
                    files_autocomplete_load_in_flight = false;
                    self.files_autocomplete.apply_loaded_files(files);
                    self.refresh_files_autocomplete_from_editor();
                    self.render();
                }
                // Git branch footer refresh tick.
                _ = git_branch_interval.tick() => {
                    // The git branch can change while running (checkout from another
                    // shell or the agent); refresh periodically off the UI loop so a
                    // slow repo can't block input/render and the footer stays current.
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

    // ── Input handling ────────────────────────────────────────────────

    async fn process_stdin_bytes(
        &mut self,
        bytes: Vec<u8>,
        stdin_rx: &mut mpsc::Receiver<Vec<u8>>,
        escape_timeout: Duration,
        kitty_fallback_done: &mut bool,
    ) -> bool {
        // Check for Kitty protocol response before buffering.
        if !self.kitty.active && !*kitty_fallback_done {
            if let Some(_flags) = KittyProtocol::parse_response(&bytes) {
                self.kitty.enable();
                *kitty_fallback_done = true;
                return false;
            }
        }

        // Feed bytes into the StdinBuffer. Kitty release events are filtered per
        // decoded sequence in `process_key_sequence` (filtering the raw read could
        // drop a press when a press+release arrive together).
        self.stdin_buffer.feed(&bytes);
        self.drain_complete_key_sequences();
        self.drain_pending_stdin_bytes(stdin_rx, escape_timeout)
            .await;
        true
    }

    async fn drain_pending_stdin_bytes(
        &mut self,
        stdin_rx: &mut mpsc::Receiver<Vec<u8>>,
        escape_timeout: Duration,
    ) {
        if !self.stdin_buffer.has_pending() || self.should_exit {
            return;
        }

        let mut retries = 0;
        while self.stdin_buffer.has_pending() && retries < MAX_ESCAPE_RETRIES && !self.should_exit {
            retries += 1;
            match tokio::time::timeout(escape_timeout, stdin_rx.recv()).await {
                Ok(Some(more)) => {
                    self.stdin_buffer.feed(&more);
                    self.drain_complete_key_sequences();
                }
                Ok(None) => break, // Channel closed (stdin EOF).
                Err(_) => break,   // Timeout — no more data coming.
            }
        }

        // Force drain anything still pending after retries exhausted (bare
        // Escape, etc.).
        if !self.should_exit {
            let forced = self.stdin_buffer.drain_all();
            for seq in &forced {
                self.process_key_sequence(seq);
                if self.should_exit {
                    break;
                }
            }
        }
    }

    fn drain_complete_key_sequences(&mut self) {
        let complete = self.stdin_buffer.drain_complete();
        for seq in &complete {
            self.process_key_sequence(seq);
            if self.should_exit {
                break;
            }
        }
    }

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

        // Panel focus model (#802): when the side panel holds focus, keys drive
        // the panel (highlight move, digit-jump, commit, cancel) rather than the
        // editor. An open autocomplete already returned above, so Tab reaching
        // here is the focus toggle, never a completion.
        // Guard on panel visibility: if every sub-agent left while focus was
        // `Panel`, the panel handler would swallow all typing and lock out the
        // editor — reset to `Input` so the editor stays usable (#804 review).
        if matches!(self.focus, Focus::Panel) {
            if self.subagent_panel_visible() {
                self.handle_panel_focus_key(&key);
                return;
            }
            self.focus = Focus::Input;
        }
        // Tab toggles focus to the panel when it is visible (the autocomplete
        // popups consumed Tab above when active, preserving completion).
        if matches!(key, Key::Tab | Key::BackTab) && self.subagent_panel_visible() {
            self.focus = Focus::Panel;
            self.sync_panel_selection_to_active();
            return;
        }

        // Global key handlers.
        // Note: Ctrl+D is handled at the top of handle_key (unconditional exit).
        match &key {
            Key::Ctrl('c') => {
                let running = self.agent_state.is_running() || self.active_subagent_running();
                match ctrl_c_action(running, self.editor.text().is_empty()) {
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
                // Parity: Esc stops the viewed agent if running, else back to master.
                if self.active_agent_id.is_some() {
                    self.last_idle_escape = None;
                    if self.active_subagent_running() {
                        self.handle_abort();
                    } else {
                        self.select_agent(None);
                    }
                    return;
                }
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
                self.master_session.chat.toggle_tool_expand();
                let state = if self.master_session.chat.tool_expanded {
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
                            match copy_to_clipboard(&text) {
                                Ok(()) => {
                                    let chars = text.chars().count();
                                    self.notify(
                                        &format!("Copied {} chars to clipboard", chars),
                                        NotifyLevel::Success,
                                    );
                                }
                                Err(error) => {
                                    self.notify(
                                        &format!("Failed to copy selection to clipboard: {error}"),
                                        NotifyLevel::Error,
                                    );
                                }
                            }
                        }
                    }
                }
                return;
            }
            Key::ScrollUp => {
                self.active_chat_mut().scroll_up(MOUSE_SCROLL_LINES);
                return;
            }
            Key::ScrollDown => {
                self.active_chat_mut().scroll_down(MOUSE_SCROLL_LINES);
                return;
            }
            Key::PageUp => {
                self.active_chat_mut().scroll_up(10);
                return;
            }
            Key::PageDown => {
                self.active_chat_mut().scroll_down(10);
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
            self.refresh_files_autocomplete_from_editor();
        }

        // Check if editor submitted.
        if let Some(text) = self.editor.take_submit() {
            self.autocomplete.dismiss();
            self.handle_submit(&text);
        }
    }

    /// Handle a key while the side panel holds focus (#802). Movement keys only
    /// move the highlight; digits 1–9 jump to a numbered row; Enter/Tab commit
    /// the highlighted agent and return focus to the input; Esc/BackTab cancel
    /// back to the input without changing the active session.
    fn handle_panel_focus_key(&mut self, key: &Key) {
        match key {
            Key::Up | Key::Char('k') => self.panel_highlight_previous(),
            Key::Down | Key::Char('j') => self.panel_highlight_next(),
            Key::Char(c @ '1'..='9') => {
                self.panel_highlight_row(*c as usize - '0' as usize);
            }
            Key::Enter | Key::Tab => {
                self.commit_panel_selection();
                self.focus = Focus::Input;
            }
            Key::Escape | Key::BackTab => {
                // Cancel: return to the input, restore the highlight to the
                // (unchanged) active session.
                self.focus = Focus::Input;
                self.sync_panel_selection_to_active();
            }
            _ => {}
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
        // Abort targets the ACTIVE session (#802): a selected sub-agent's abort
        // is routed over its own connection and finalizes its transcript, never
        // touching the master's run state.
        if self.active_agent_id.is_some() {
            self.send_to_active_subagent(Command::Abort { id: None });
            self.active_chat_mut().finalize_assistant();
            if let Some(id) = self.active_agent_id.clone() {
                if let Some(session) = self.sessions.get_mut(&id) {
                    session.running = false;
                }
            }
            return;
        }

        // Send abort to agent.
        self.send_command(Command::Abort { id: None });

        // Call abort on the state machine — does NOT set running to false.
        // The AgentEnd event will arrive and be matched against the current
        // generation, preventing stale events from corrupting state (#502).
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
