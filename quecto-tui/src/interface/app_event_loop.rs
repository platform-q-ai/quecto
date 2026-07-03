use super::app_selection::{SelectionAnchor, TextSelection};
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StreamRenderDecision {
    RenderNow,
    DeferUntil(tokio::time::Instant),
}

#[derive(Debug, Default)]
pub(super) struct StreamRenderCoalescer {
    last_render_at: Option<tokio::time::Instant>,
    pending_deadline: Option<tokio::time::Instant>,
}

fn mark_stream_render_complete(
    coalescer: &mut StreamRenderCoalescer,
    next_deadline: &mut Option<tokio::time::Instant>,
) {
    coalescer.note_immediate_render(tokio::time::Instant::now());
    *next_deadline = None;
}

impl StreamRenderCoalescer {
    pub(super) fn record_token_update(
        &mut self,
        now: tokio::time::Instant,
    ) -> StreamRenderDecision {
        let Some(last_render_at) = self.last_render_at else {
            self.last_render_at = Some(now);
            return StreamRenderDecision::RenderNow;
        };

        let next_frame = last_render_at + STREAM_RENDER_INTERVAL;
        if now >= next_frame {
            self.last_render_at = Some(now);
            self.pending_deadline = None;
            StreamRenderDecision::RenderNow
        } else {
            self.pending_deadline = Some(next_frame);
            StreamRenderDecision::DeferUntil(next_frame)
        }
    }

    pub(super) fn render_due(&mut self, now: tokio::time::Instant) -> bool {
        let Some(deadline) = self.pending_deadline else {
            return false;
        };
        if now < deadline {
            return false;
        }
        self.pending_deadline = None;
        self.last_render_at = Some(now);
        true
    }

    pub(super) fn note_immediate_render(&mut self, now: tokio::time::Instant) {
        self.pending_deadline = None;
        self.last_render_at = Some(now);
    }
}

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

        let mut stream_render_coalescer = StreamRenderCoalescer::default();
        let mut next_stream_render_deadline: Option<tokio::time::Instant> = None;

        // Initial render.
        self.render();
        mark_stream_render_complete(
            &mut stream_render_coalescer,
            &mut next_stream_render_deadline,
        );

        let mut next_animation_tick = tokio::time::Instant::now() + SPINNER_TICK;

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
            let next_idle_service_tick = self.next_idle_service_deadline();

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
                    mark_stream_render_complete(
                        &mut stream_render_coalescer,
                        &mut next_stream_render_deadline,
                    );
                }
                // Agent events.
                event = self.client.recv(), if self.agent_connected => {
                    match event {
                        Some(ev) => {
                            let is_token = matches!(ev, Event::Token { .. });
                            self.handle_event(ev);
                            if is_token {
                                match stream_render_coalescer.record_token_update(tokio::time::Instant::now()) {
                                    StreamRenderDecision::RenderNow => {
                                        self.render();
                                        next_stream_render_deadline = None;
                                    }
                                    StreamRenderDecision::DeferUntil(deadline) => {
                                        next_stream_render_deadline = Some(deadline);
                                    }
                                }
                            } else {
                                self.render();
                                mark_stream_render_complete(
                                    &mut stream_render_coalescer,
                                    &mut next_stream_render_deadline,
                                );
                            }
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
                            mark_stream_render_complete(
                                &mut stream_render_coalescer,
                                &mut next_stream_render_deadline,
                            );
                        }
                    }
                }
                // Terminal resize.
                Some(()) = resize_rx.recv() => {
                    self.terminal.refresh_size();
                    self.render_full();
                    mark_stream_render_complete(
                        &mut stream_render_coalescer,
                        &mut next_stream_render_deadline,
                    );
                }
                // Animation / fallback tick.
                _ = tokio::time::sleep_until(next_animation_tick), if self.needs_animation_tick(!kitty_fallback_done) => {
                    let needs_render = self.service_animation_tick(&mut kitty_fallback_done, kitty_deadline);
                    next_animation_tick = tokio::time::Instant::now() + SPINNER_TICK;
                    if needs_render {
                        self.render();
                        mark_stream_render_complete(
                            &mut stream_render_coalescer,
                            &mut next_stream_render_deadline,
                        );
                    }
                }
                // One-shot idle service deadline for static notification expiry and exited-subagent GC.
                _ = async {
                    if let Some(deadline) = next_idle_service_tick {
                        tokio::time::sleep_until(deadline).await;
                    }
                }, if next_idle_service_tick.is_some() && !self.needs_animation_tick(!kitty_fallback_done) => {
                    let needs_render = self.service_animation_tick(&mut kitty_fallback_done, kitty_deadline);
                    if needs_render {
                        self.render();
                        mark_stream_render_complete(
                            &mut stream_render_coalescer,
                            &mut next_stream_render_deadline,
                        );
                    }
                }
                Some(branch) = git_branch_rx.recv() => {
                    git_branch_refresh_in_flight = false;
                    if self.apply_git_branch(branch) {
                        self.render();
                        mark_stream_render_complete(
                            &mut stream_render_coalescer,
                            &mut next_stream_render_deadline,
                        );
                    }
                }
                Some(failure) = self.command_send_failure_rx.recv() => {
                    self.handle_command_send_failure(failure);
                    self.render();
                    mark_stream_render_complete(
                        &mut stream_render_coalescer,
                        &mut next_stream_render_deadline,
                    );
                }
                // Events fanned in from the active sub-agent's direct
                // connect-on-select connection (#800).
                Some((agent_id, ev)) = self.subagent_event_rx.recv() => {
                    self.route_subagent_event(&agent_id, ev);
                    self.render();
                    mark_stream_render_complete(
                        &mut stream_render_coalescer,
                        &mut next_stream_render_deadline,
                    );
                }
                Some(files) = files_autocomplete_rx.recv() => {
                    files_autocomplete_load_in_flight = false;
                    self.files_autocomplete.apply_loaded_files(files);
                    self.refresh_files_autocomplete_from_editor();
                    self.render();
                    mark_stream_render_complete(
                        &mut stream_render_coalescer,
                        &mut next_stream_render_deadline,
                    );
                }
                _ = async {
                    if let Some(deadline) = next_stream_render_deadline {
                        tokio::time::sleep_until(deadline).await;
                    }
                }, if next_stream_render_deadline.is_some() => {
                    if stream_render_coalescer.render_due(tokio::time::Instant::now()) {
                        self.render();
                    }
                    next_stream_render_deadline = None;
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

    pub(super) async fn process_stdin_bytes(
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
                // Toggle tool output expansion for the ACTIVE session (#828).
                self.active_chat_mut().toggle_tool_expand();
                let state = if self.active_chat_mut().tool_expanded {
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
}
