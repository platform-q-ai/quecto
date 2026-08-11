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

impl StreamRenderCoalescer {
    /// Deadline of a deferred token paint, if one is pending. The event loop's
    /// deadline select arm derives from this single source of truth so loop
    /// state can never drift from the coalescer (#1011 review).
    pub(super) fn pending_deadline(&self) -> Option<tokio::time::Instant> {
        self.pending_deadline
    }

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

/// How the event loop should paint after routing one fan-in item (#1462):
/// the exact render decision each replaced select arm made before the seam.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SourcedRender {
    /// Paint now and re-base the coalescer (disconnects, surfaced drops).
    Immediate,
    /// Stream render: token events coalesce, everything else paints now.
    Stream { is_token: bool },
    /// No paint from this item: either nothing was routed (a payload-less
    /// non-sentinel item), or a `Source::Closed` sentinel deferred its
    /// disconnect diagnosis off-loop — the paint happens when the diagnosis
    /// lands on the disconnect channel's own select arm.
    Skip,
}

impl App {
    /// Route one item drained from the shared fan-in channel (#1462): master
    /// events (`Source::Tab`) go through the master event handler, sub-agent
    /// events (`Source::Subagent`) through sub-agent routing, and the
    /// `Source::Closed` sentinel runs the #1047 disconnect diagnosis path.
    ///
    /// The `Source::Closed` arm never awaits the child-exit diagnosis here
    /// (#1462 scope 3): when the TUI owns the agent child, the bounded
    /// #1047 waits run on a spawned task and complete through the
    /// disconnect-diagnosis select arm — a dying child cannot stall event
    /// processing while other connections' events are queued.
    pub(super) async fn route_sourced(
        &mut self,
        source: crate::shell::connection::Source,
        ev: Option<Event>,
    ) -> SourcedRender {
        use crate::shell::connection::Source;
        match (source, ev) {
            (Source::Tab(_), Some(ev)) => {
                let is_token = Self::is_token_event(&ev);
                self.handle_event(ev);
                if self.surface_dropped_oversized_events() {
                    SourcedRender::Immediate
                } else {
                    SourcedRender::Stream { is_token }
                }
            }
            (Source::Subagent(_, agent_id), Some(ev)) => {
                let is_token = Self::is_token_event(&ev);
                self.route_subagent_event(&agent_id, ev);
                SourcedRender::Stream { is_token }
            }
            (Source::Closed(_), _) => {
                // Master stream closed — dispatch the child-exit diagnosis
                // off-loop (#1047 via #1462 scope 3). Without an owned child
                // the disconnect (incl. dropped-event surfacing) completes
                // synchronously and paints now; with one, the paint happens
                // when the diagnosis lands on its select arm.
                if self.begin_agent_stream_closed() {
                    SourcedRender::Skip
                } else {
                    SourcedRender::Immediate
                }
            }
            (Source::Tab(_) | Source::Subagent(..), None) => SourcedRender::Skip,
        }
    }

    /// Render immediately and note it on the coalescer so a pending deferred
    /// token paint is consumed by this render (it paints all accumulated
    /// state, including any deferred tokens).
    pub(super) fn render_and_note(&mut self, coalescer: &mut StreamRenderCoalescer) {
        self.render();
        coalescer.note_immediate_render(tokio::time::Instant::now());
    }

    /// Whether an agent event is a streaming token (the only event class the
    /// coalescer defers). Single source of truth for the event-loop arms and
    /// the test harness so classification cannot drift between them.
    pub(super) fn is_token_event(ev: &Event) -> bool {
        matches!(ev, Event::Token { .. })
    }

    /// Render for an incoming agent event: token events are coalesced to the
    /// stream frame interval; everything else renders immediately.
    pub(super) fn render_stream_event(
        &mut self,
        coalescer: &mut StreamRenderCoalescer,
        is_token: bool,
    ) {
        if !is_token {
            self.render_and_note(coalescer);
            return;
        }
        if let StreamRenderDecision::RenderNow =
            coalescer.record_token_update(tokio::time::Instant::now())
        {
            self.render();
        }
    }

    pub async fn run(&mut self) -> i32 {
        self.terminal.enter_raw_mode();
        self.terminal.hide_cursor();

        // Query Kitty keyboard protocol support.
        self.kitty.query();

        // Query initial state from agent through the shared command path so
        // startup send failures surface in the UI like user-initiated sends.
        self.send_command(Command::GetState {
            agent_id: None,
            id: Some("init".into()),
        });
        // Query initial subagent state (#525).
        self.send_command(Command::GetSubagents {
            id: Some("init-subagents".into()),
        });
        // Backfill durable master history on connect so `--socket` attach (and
        // any reconnect) shows prior session content without waiting for new
        // events. Empty payloads do not latch the guard (#1050 / #828).
        self.request_master_attach_backfill();

        // Set up SIGWINCH handler.
        let mut resize_rx = crate::shell::signals::sigwinch_stream().await;

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

        // Initial render.
        self.render_and_note(&mut stream_render_coalescer);

        let mut next_animation_tick = tokio::time::Instant::now() + SPINNER_TICK;

        // Git branch footer refresh timer.
        let mut git_branch_interval = tokio::time::interval(app_git::GIT_BRANCH_POLL_INTERVAL);
        git_branch_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let (git_branch_tx, mut git_branch_rx) = mpsc::channel::<Option<String>>(1);
        let mut git_branch_refresh_in_flight = false;

        // @files autocomplete workspace enumeration. Loading can shell out to git
        // or walk the filesystem, so keep it off the input/render loop.
        let (files_autocomplete_tx, mut files_autocomplete_rx) =
            mpsc::channel::<(PathBuf, Vec<String>)>(1);
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

                    if self.workspace.files_autocomplete.take_load_request() {
                        self.start_files_autocomplete_load(
                            &files_autocomplete_tx,
                            &mut files_autocomplete_load_in_flight,
                        );
                    }
                    self.render_and_note(&mut stream_render_coalescer);
                }
                // Terminal resize.
                Some(()) = resize_rx.recv() => {
                    self.terminal.refresh_size();
                    self.render_full();
                    stream_render_coalescer.note_immediate_render(tokio::time::Instant::now());
                }
                // Animation / fallback tick.
                _ = tokio::time::sleep_until(next_animation_tick), if self.needs_animation_tick(!kitty_fallback_done) => {
                    let needs_render = self.service_animation_tick(&mut kitty_fallback_done, kitty_deadline);
                    next_animation_tick = tokio::time::Instant::now() + SPINNER_TICK;
                    if needs_render {
                        self.render_and_note(&mut stream_render_coalescer);
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
                        self.render_and_note(&mut stream_render_coalescer);
                    }
                }
                Some(branch) = git_branch_rx.recv() => {
                    git_branch_refresh_in_flight = false;
                    if self.apply_git_branch(branch) {
                        self.render_and_note(&mut stream_render_coalescer);
                    }
                }
                // Off-loop disconnect diagnosis completion (#1462 scope 3):
                // the bounded #1047 waits ran on a spawned task; finish the
                // disconnect with the diagnosis it reported.
                Some(detail) = self.disconnect_diag_rx.recv() => {
                    self.finish_agent_stream_closed(detail);
                    self.render_and_note(&mut stream_render_coalescer);
                }
                Some(failure) = self.command_send_failure_rx.recv() => {
                    self.handle_command_send_failure(failure);
                    self.render_and_note(&mut stream_render_coalescer);
                }
                // ONE shared fan-in for every connection (#1462): the master
                // connection's feed task (`Source::Tab` / `Source::Closed`)
                // and the per-subagent feeds (`Source::Subagent`). The select
                // arm count stays constant as connections come and go.
                Some((source, ev)) = self.subagents.event_rx.recv() => {
                    match self.route_sourced(source, ev).await {
                        SourcedRender::Immediate => {
                            self.render_and_note(&mut stream_render_coalescer);
                        }
                        SourcedRender::Stream { is_token } => {
                            self.render_stream_event(&mut stream_render_coalescer, is_token);
                        }
                        SourcedRender::Skip => {}
                    }
                }
                Some((root, files)) = files_autocomplete_rx.recv() => {
                    files_autocomplete_load_in_flight = false;
                    if self.apply_files_autocomplete_load(root, files) {
                        self.render_and_note(&mut stream_render_coalescer);
                    }
                }
                // Deferred stream paint: fires at the coalescer's pending frame
                // deadline so a stalled burst still gets its final paint.
                //
                // Subtlety: `select!` re-creates this `async {}` future on every
                // loop iteration, so `pending_deadline()` is re-read each time —
                // if a token/immediate render moves or clears the deadline, the
                // next iteration arms the sleep against the NEW deadline (or
                // disarms via the `if` guard). Correct, but only because the
                // deadline is re-read on re-poll; do not hoist the sleep out.
                _ = async {
                    if let Some(deadline) = stream_render_coalescer.pending_deadline() {
                        tokio::time::sleep_until(deadline).await;
                    }
                }, if stream_render_coalescer.pending_deadline().is_some() => {
                    if stream_render_coalescer.render_due(tokio::time::Instant::now()) {
                        self.render();
                    }
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

    /// Process a single complete key sequence from the StdinBuffer.
    pub(super) fn process_key_sequence(&mut self, seq: &[u8]) {
        // Kitty key release events can arrive in the same stdin read as a key
        // press. Filtering only the raw read can miss them, causing one arrow
        // key press to be handled twice. Drop releases per decoded sequence.
        if self.kitty.active && crate::components::kitty::is_key_release(seq) {
            return;
        }
        if let Some((key, _)) = keys::parse_key(seq) {
            self.handle_key(key);
        }
    }

    pub(super) fn handle_key(&mut self, key: Key) {
        if !matches!(key, Key::Escape) {
            self.rewind.last_idle_escape = None;
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
        if self.sessions.resume_selector.is_some() {
            self.rewind.last_idle_escape = None;
            self.handle_resume_selector_key(&key);
            return;
        }

        // If the rewind selector is active, route input to it.
        if self.rewind.selector.is_some() {
            self.rewind.last_idle_escape = None;
            self.handle_rewind_selector_key(&key);
            return;
        }

        // If the tool policy modal is active, route input to it.
        if self.tool_policy_modal.is_some() {
            self.rewind.last_idle_escape = None;
            self.handle_tool_policy_modal_key(&key);
            return;
        }

        // If the model selector is active, route input to it.
        if self.inference.model_selector.is_some() {
            self.rewind.last_idle_escape = None;
            self.handle_model_selector_key(&key);
            return;
        }

        // If the effort selector is active, route input to it (#1067).
        if self.inference.effort_selector.is_some() {
            self.rewind.last_idle_escape = None;
            self.handle_effort_selector_key(&key);
            return;
        }

        // If autocomplete is active, route navigation keys there.
        if self.autocomplete.is_active() {
            match &key {
                Key::Up | Key::Down | Key::Tab | Key::Escape => {
                    if matches!(key, Key::Escape) {
                        self.rewind.last_idle_escape = None;
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
        if self.workspace.files_autocomplete.is_active() {
            match &key {
                Key::Up | Key::Down | Key::Tab | Key::Escape => {
                    self.workspace.files_autocomplete.handle_input(&key);
                    if let AutocompleteResult::Selected(path) =
                        self.workspace.files_autocomplete.take_result()
                    {
                        self.accept_file_mention(&path);
                    }
                    return;
                }
                Key::Enter => {
                    self.workspace.files_autocomplete.handle_input(&Key::Tab);
                    if let AutocompleteResult::Selected(path) =
                        self.workspace.files_autocomplete.take_result()
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
        if matches!(self.subagents.focus, Focus::Panel) {
            if self.subagent_panel_visible() {
                self.handle_panel_focus_key(&key);
                return;
            }
            self.subagents.focus = Focus::Input;
        }
        // Tab toggles focus to the panel when it is visible (the autocomplete
        // popups consumed Tab above when active, preserving completion).
        if matches!(key, Key::Tab | Key::BackTab) && self.subagent_panel_visible() {
            self.subagents.focus = Focus::Panel;
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
                if self.subagents.active_agent_id.is_some() {
                    self.rewind.last_idle_escape = None;
                    if self.active_subagent_running() {
                        self.handle_abort();
                    } else {
                        self.select_agent(None);
                    }
                    return;
                }
                if self.agent_state.is_running() {
                    self.rewind.last_idle_escape = None;
                    self.handle_abort();
                } else if !self.editor.text().is_empty() {
                    self.rewind.last_idle_escape = None;
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
                crate::shell::signals::suspend();
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
            Key::Ctrl('t') => {
                self.open_tool_policy_modal();
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
                self.request_active_older_history_page();
                self.request_active_visible_stub_recalls();
                return;
            }
            Key::ScrollDown => {
                self.active_chat_mut().scroll_down(MOUSE_SCROLL_LINES);
                return;
            }
            Key::PageUp => {
                self.active_chat_mut().scroll_up(10);
                self.request_active_older_history_page();
                self.request_active_visible_stub_recalls();
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
            self.workspace.files_autocomplete.dismiss();
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
            Key::ScrollUp | Key::PageUp => self.panel_highlight_previous_by(MOUSE_SCROLL_LINES),
            Key::ScrollDown | Key::PageDown => self.panel_highlight_next_by(MOUSE_SCROLL_LINES),
            Key::Char(c @ '1'..='9') => {
                self.panel_highlight_row(*c as usize - '0' as usize);
            }
            Key::Enter | Key::Tab => {
                self.commit_panel_selection();
                self.subagents.focus = Focus::Input;
            }
            Key::Escape | Key::BackTab => {
                // Cancel: return to the input, restore the highlight to the
                // (unchanged) active session.
                self.subagents.focus = Focus::Input;
                self.sync_panel_selection_to_active();
            }
            _ => {}
        }
    }

    /// Replace the active `@token` in the editor with the selected file path.
    fn accept_file_mention(&mut self, path: &str) {
        if let Some(start) = self.workspace.files_autocomplete.token_start() {
            self.editor
                .replace_before_cursor(start, &format!("@{path} "));
        }
        self.workspace.files_autocomplete.dismiss();
    }
}
