// Reusable test toolkit: not every helper is exercised by the current tests.
#![allow(dead_code)]
//! Headless render harness for TUI layout/flicker testing.
//!
//! Drives the *real* render path (`App::compose_bottom`) with scripted events
//! and a fixed terminal size — no TTY, no live agent — and captures the
//! below-chat section frame-by-frame. Tests assert on layout stability (no
//! transient "flash" lines, bounded height changes) so TUI regressions like the
//! sub-agent panel judder can be reproduced and caught in `cargo test` without
//! manually eyeballing a live session.
//!
//! The below-chat section (sub-agent panel → workflow bar → spinner → editor →
//! notifications → footer) is exactly the region whose height changes reflow
//! the chat, so that's what the harness records.

use super::App;
use super::Focus;
use super::app_methods::strip_ansi;
use super::app_selection::{SelectionAnchor, TextSelection};
use super::keys::Key;
use crate::infrastructure::client::{Client, Event};
use crate::infrastructure::terminal::Terminal;
use crate::interface::components::chat::ChatEntry;
use crate::interface::components::notification::NotifyLevel;
use crate::interface::components::spinner::Spinner;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

#[path = "tui_harness_events.rs"]
mod events;
// Re-export the scenario event builders so callers keep using
// `tui_harness::subagent(..)` etc. `normalize` stays module-internal.
use events::normalize;
pub use events::*;

const DEFAULT_WIDTH: usize = 120;
const DEFAULT_HEIGHT: usize = 40;

static SEQ: AtomicU64 = AtomicU64::new(0);

/// Build an App with a fixed terminal size and a dummy (drained) socket client,
/// so the render path runs headlessly and deterministically.
async fn headless_app(width: usize, height: usize) -> (App, mpsc::Receiver<String>) {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("quecto-tui-harness-{}-{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let socket_path = dir.join("agent.sock");
    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
    let (cmd_tx, cmd_rx) = mpsc::channel(64);
    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            let mut lines = BufReader::new(stream).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if cmd_tx.send(line).await.is_err() {
                    break;
                }
            }
        }
    });
    let client = Client::connect(&socket_path).await.unwrap();
    let mut term = Terminal::new();
    term.width = width;
    term.height = height;
    (App::new(term, client), cmd_rx)
}

/// Frame-capturing harness over the real render path.
pub struct TuiHarness {
    app: App,
    width: usize,
    /// Below-chat section per captured frame (ANSI-stripped, trailing-trimmed).
    bottoms: Vec<Vec<String>>,
    /// Full screen frame per captured frame (ANSI-stripped) — for catching
    /// chat-area transients (e.g. a tool result that flashes in and out).
    fulls: Vec<Vec<String>>,
    cmd_rx: mpsc::Receiver<String>,
    /// Stream render coalescer driven by the event-loop-path helpers (#972).
    stream_coalescer: super::app_event_loop::StreamRenderCoalescer,
}

impl TuiHarness {
    pub async fn new() -> Self {
        Self::sized(DEFAULT_WIDTH, DEFAULT_HEIGHT).await
    }

    pub async fn sized(width: usize, height: usize) -> Self {
        let (app, cmd_rx) = headless_app(width, height).await;
        Self {
            app,
            width,
            bottoms: Vec::new(),
            fulls: Vec::new(),
            cmd_rx,
            stream_coalescer: super::app_event_loop::StreamRenderCoalescer::default(),
        }
    }

    /// Feed an event through the real handler and capture the resulting frame.
    pub fn event(&mut self, ev: Event) -> &mut Self {
        self.app.handle_event(ev);
        self.capture();
        self
    }

    /// Feed a raw wire JSON line through the REAL `Event` deserializer (the path
    /// `event()` skips). Panics if the line fails to parse — the connected
    /// client would otherwise drop it (and historically printed it over the
    /// TUI), so a forwarded/edge event that doesn't deserialize is a bug the
    /// render-only `event()` API can't catch.
    pub fn event_line(&mut self, json: &str) -> &mut Self {
        let ev: Event = serde_json::from_str(json)
            .unwrap_or_else(|e| panic!("wire event failed to deserialize: {e}\n  line: {json}"));
        self.event(ev)
    }

    /// Advance the spinner/elapsed animation tick and capture.
    pub fn tick(&mut self) -> &mut Self {
        self.app.tick_subagent_animation();
        self.capture();
        self
    }

    /// Run the exited-sub-agent GC and capture.
    pub fn gc(&mut self) -> &mut Self {
        self.app.gc_exited_subagents();
        self.capture();
        self
    }

    /// Capture the current below-chat section and the full frame (ANSI-stripped).
    pub fn capture(&mut self) {
        let w = self.width;
        let bottom = self
            .app
            .compose_bottom(w)
            .iter()
            .map(|l| strip_ansi(l).trim_end().to_string())
            .collect();
        self.bottoms.push(bottom);
        let full = self
            .app
            .compose_frame()
            .iter()
            .map(|l| strip_ansi(l).trim_end().to_string())
            .collect();
        self.fulls.push(full);
    }

    /// Frame-by-frame dump of the FULL screen (for `--nocapture`).
    pub fn dump_full(&self) -> String {
        let mut s = String::new();
        for (i, f) in self.fulls.iter().enumerate() {
            s.push_str(&format!("=== full frame {i} ===\n"));
            for l in f {
                if l.trim().is_empty() {
                    continue;
                }
                s.push_str("  | ");
                s.push_str(l);
                s.push('\n');
            }
        }
        s
    }

    // ── Analysis ──────────────────────────────────────────────────────

    /// Below-chat line count per captured frame.
    pub fn heights(&self) -> Vec<usize> {
        self.bottoms.iter().map(|b| b.len()).collect()
    }

    /// Frames where the below-chat height spikes up-then-down or dips
    /// down-then-up — a visible single-frame flash. Returns (frame, prev, cur, next).
    pub fn flashes(&self) -> Vec<(usize, usize, usize, usize)> {
        let h = self.heights();
        let mut out = Vec::new();
        if h.len() < 3 {
            return out;
        }
        for i in 1..h.len() - 1 {
            let (p, c, n) = (h[i - 1], h[i], h[i + 1]);
            if (c > p && c > n) || (c < p && c < n) {
                out.push((i, p, c, n));
            }
        }
        out
    }

    /// Frame-by-frame dump of the below-chat section (for `--nocapture`).
    pub fn dump(&self) -> String {
        let mut s = String::new();
        for (i, b) in self.bottoms.iter().enumerate() {
            s.push_str(&format!(
                "--- frame {i} ({} below-chat lines) ---\n",
                b.len()
            ));
            for l in b {
                s.push_str("  | ");
                s.push_str(l);
                s.push('\n');
            }
        }
        s
    }

    /// The most recently captured below-chat section, joined into one string.
    pub fn last(&self) -> String {
        self.bottoms
            .last()
            .map(|b| b.join("\n"))
            .unwrap_or_default()
    }

    /// Escape hatch for driving fields the event API doesn't cover.
    pub fn app_mut(&mut self) -> &mut App {
        &mut self.app
    }

    /// Whether the event loop would currently arm its sub-second animation timer.
    pub fn needs_animation_tick(&self, kitty_fallback_pending: bool) -> bool {
        self.app.needs_animation_tick(kitty_fallback_pending)
    }

    /// Drive one animation/fallback service tick.
    pub fn service_animation_tick(
        &mut self,
        kitty_fallback_done: &mut bool,
        kitty_deadline: tokio::time::Instant,
    ) -> bool {
        self.app
            .service_animation_tick(kitty_fallback_done, kitty_deadline)
    }

    /// Capture the current spinner frame index, when a spinner is visible.
    pub fn spinner_frame_index(&self) -> Option<usize> {
        self.app.spinner.as_ref().map(Spinner::frame_index)
    }

    /// Mark the main session's streaming indicator.
    pub fn set_streaming(&mut self, streaming: bool) {
        self.app.master_session.footer.set_streaming(streaming);
    }

    /// Mark the main agent as not running.
    pub fn end_agent_run(&mut self) {
        self.app.agent_state.end();
    }

    /// Show a visible activity spinner in the main session.
    pub fn show_activity_spinner(&mut self, message: &str) {
        self.app.spinner = Some(Spinner::new(message));
    }

    /// Show a visible notification.
    pub fn notify(&mut self, message: &str) {
        self.app.notify(message, NotifyLevel::Info);
    }

    /// Whether any notification is still visible.
    pub fn has_notification(&self) -> bool {
        !self.app.notifications.is_empty()
    }

    /// Force the footer branch value through the production apply helper.
    pub fn apply_branch(&mut self, branch: Option<String>) -> bool {
        self.app.apply_git_branch(branch)
    }

    /// Override the repository root used by git branch refresh tests.
    pub fn set_git_repo(&mut self, repo: std::path::PathBuf) {
        self.app.git_repo = Some(repo);
    }

    /// Drive the same branch refresh task used by the event-loop interval.
    pub async fn refresh_branch_from_repo(&mut self) -> bool {
        let (tx, mut rx) = mpsc::channel(1);
        let mut in_flight = false;
        self.app.start_git_branch_refresh(&tx, &mut in_flight);
        drop(tx);
        if let Some(branch) = rx.recv().await {
            self.app.apply_git_branch(branch)
        } else {
            false
        }
    }

    /// Feed printable input through the real key handler.
    pub fn type_char(&mut self, ch: char) {
        self.app.handle_key(Key::Char(ch));
    }

    /// Current editor text.
    pub fn editor_text(&self) -> String {
        self.app.editor.text().to_string()
    }

    /// Mark Kitty protocol support as absent for fallback assertions.
    pub fn clear_kitty_support(&mut self) {
        self.app.kitty.active = false;
        self.app.kitty.modify_other_keys = false;
    }

    /// Whether modifyOtherKeys fallback is enabled.
    pub fn modify_other_keys_enabled(&self) -> bool {
        self.app.kitty.modify_other_keys
    }

    /// Enable the xterm modifyOtherKeys fallback through the real protocol path.
    pub fn enable_modify_other_keys(&mut self) {
        self.app.kitty.enable_modify_other_keys();
    }

    /// Run the real on-exit terminal protocol cleanup the event loop performs
    /// during teardown (`KittyProtocol::cleanup` — pops Kitty flags and resets
    /// modifyOtherKeys to mode 0).
    pub fn run_protocol_cleanup(&mut self) {
        self.app.kitty.cleanup();
    }

    /// Whether any notification is currently visible, rendered to text via the
    /// real notification stack so tests can assert the message content.
    pub fn notification_text(&mut self) -> String {
        use crate::interface::component::Component;
        let w = self.width;
        self.app.notifications.render(w).join("\n")
    }

    /// Feed a real `io::Error` through the production render-failure handler
    /// (`handle_render_failure` — invalidate + error notification) and return
    /// the resulting notification text.
    pub fn handle_render_failure(&mut self, error: &std::io::Error) -> String {
        self.app.handle_render_failure(error);
        self.notification_text()
    }

    /// Replace the agent client with a disconnected one and drive the real
    /// `send_command` path; on the expected send failure, route it through the
    /// production `handle_command_send_failure` handler and return the
    /// resulting error-notification text.
    pub async fn send_command_expecting_failure(
        &mut self,
        cmd: crate::infrastructure::client::Command,
    ) -> String {
        self.app.client = Client::disconnected_for_tests();
        self.app.send_command(cmd);
        let failure = self
            .app
            .command_send_failure_rx
            .recv()
            .await
            .expect("command send failure should be routed to the app");
        self.app.handle_command_send_failure(failure);
        self.notification_text()
    }

    /// Whether the real stdin buffer still holds pending (incomplete) bytes.
    pub fn stdin_has_pending(&self) -> bool {
        self.app.stdin_buffer.has_pending()
    }

    /// Drive the REAL multi-fragment escape retry loop (`process_stdin_bytes`)
    /// with an initial chunk plus pre-queued follow-up fragments, using the
    /// real 10ms escape timeout and `MAX_ESCAPE_RETRIES` cap. Returns the count
    /// of follow-up fragments the loop left unconsumed in the channel (a
    /// non-zero leftover proves the retry cap stopped the loop early).
    pub async fn drive_stdin_retry_loop(&mut self, first: &[u8], followups: &[&[u8]]) -> usize {
        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(64);
        for f in followups {
            tx.send(f.to_vec()).await.expect("queue stdin fragment");
        }
        let mut kitty_fallback_done = true;
        self.app
            .process_stdin_bytes(
                first.to_vec(),
                &mut rx,
                std::time::Duration::from_millis(10),
                &mut kitty_fallback_done,
            )
            .await;
        let mut leftover = 0;
        while rx.try_recv().is_ok() {
            leftover += 1;
        }
        leftover
    }

    // ── High-level driving surface for the workspace `bdd` target (#805) ──
    // These wrap crate-internal `App` methods so external integration tests can
    // drive the real render/key path without touching `pub(super)` internals.

    /// Feed one key through the real key handler and capture the frame.
    pub fn press(&mut self, key: Key) -> &mut Self {
        self.app.handle_key(key);
        self.capture();
        self
    }

    /// Select an agent's session (`None` = master) and capture the frame.
    pub fn select(&mut self, agent_id: Option<&str>) -> &mut Self {
        self.app.select_agent(agent_id);
        self.capture();
        self
    }

    /// Route one event from a sub-agent's direct connection into its session.
    pub fn route(&mut self, agent_id: &str, ev: Event) -> &mut Self {
        self.app.route_subagent_event(agent_id, ev);
        self.capture();
        self
    }

    /// Submit a prompt through the real submit path (steers the active session).
    pub fn submit(&mut self, text: &str) -> &mut Self {
        self.app.handle_submit(text);
        self.capture();
        self
    }

    /// Whether the slash-command autocomplete dropdown is currently active.
    pub fn autocomplete_active(&self) -> bool {
        self.app.autocomplete.is_active()
    }

    /// The 0-based highlighted index in the slash-command autocomplete.
    pub fn autocomplete_selected_index(&self) -> usize {
        self.app.autocomplete.selected_index()
    }

    /// The `value` of the highlighted slash-command suggestion (e.g. `"/quit"`).
    pub fn autocomplete_selected_value(&self) -> Option<String> {
        self.app.autocomplete.selected_value()
    }

    /// The number of suggestions the slash-command autocomplete currently holds.
    pub fn autocomplete_suggestion_count(&self) -> usize {
        self.app.autocomplete.suggestion_count()
    }

    /// Re-run the slash-command autocomplete `update` against the current editor
    /// text — the exact call `handle_key` makes after every editor change.
    pub fn refresh_autocomplete(&mut self) {
        let text = self.app.editor.text();
        self.app.autocomplete.update(&text);
    }

    /// Whether the app has been asked to exit (e.g. after submitting `/quit`).
    pub fn should_exit(&self) -> bool {
        self.app.should_exit
    }

    /// Whether the master agent run-state machine currently reports running.
    pub fn agent_running(&self) -> bool {
        self.app.agent_state.is_running()
    }

    /// Number of aborted runs whose stale `AgentEnd` events have not yet been
    /// consumed by the abort-aware state machine (#502/#536). Used to assert
    /// that Ctrl+C / Esc actually drove `handle_abort`.
    pub fn pending_aborts(&self) -> u32 {
        self.app.agent_state.pending_aborts
    }

    /// Set the editor text through the real editor component (the same call the
    /// production accept path makes), bypassing per-key autocomplete side effects.
    pub fn set_editor_text(&mut self, text: &str) {
        self.app.editor.set_text(text);
    }

    /// Open the model-selector overlay synchronously (production `open_model_
    /// selector_now`), so overlay-active key routing can be exercised without a
    /// socket round-trip for the model list.
    pub fn open_overlay(&mut self) {
        self.app.open_model_selector_now();
    }

    /// Whether the model-selector overlay is currently open (e.g. after
    /// submitting `/model` with no argument).
    pub fn model_selector_open(&self) -> bool {
        self.app.model_selector.is_some()
    }

    /// Built-in slash command names from the production command registry.
    pub fn slash_command_names() -> Vec<String> {
        super::builtin_commands()
            .into_iter()
            .map(|c| c.name)
            .collect()
    }

    // ── Event-loop render-path driving (#972) ─────────────────────────
    // These drive the REAL event-loop render helpers (`render_stream_event`,
    // `render_and_note`) against the App's own paint counter, so coalescing
    // is tested through production wiring rather than a re-simulation.

    /// Frames actually painted through `App::render` so far.
    pub fn rendered_frames(&self) -> usize {
        self.app.rendered_frames
    }

    /// Whether a deferred token paint is pending on the stream coalescer.
    pub fn pending_stream_paint(&self) -> bool {
        self.stream_coalescer.pending_deadline().is_some()
    }

    /// Deliver one agent event through the real event handler and the real
    /// event-loop render decision (token events coalesce; others paint now).
    pub fn stream_event(&mut self, ev: Event) -> &mut Self {
        self.app.suppress_paint = true;
        let is_token = App::is_token_event(&ev);
        self.app.handle_event(ev);
        let mut coalescer = std::mem::take(&mut self.stream_coalescer);
        self.app.render_stream_event(&mut coalescer, is_token);
        self.stream_coalescer = coalescer;
        self
    }

    /// Paint for a non-stream wakeup (stdin input, resize, tick) through the
    /// same helper the event loop's other select arms use.
    pub fn immediate_render(&mut self) -> &mut Self {
        self.app.suppress_paint = true;
        let mut coalescer = std::mem::take(&mut self.stream_coalescer);
        self.app.render_and_note(&mut coalescer);
        self.stream_coalescer = coalescer;
        self
    }

    /// Paint for a terminal-resize wakeup exactly like the event loop's resize
    /// select arm: full redraw, then re-base the coalescer so a pending
    /// deferred token paint is consumed by this render.
    pub fn resize_render(&mut self) -> &mut Self {
        self.app.suppress_paint = true;
        self.app.render_full();
        self.stream_coalescer
            .note_immediate_render(tokio::time::Instant::now());
        self
    }

    /// Fire the deferred-paint select arm: paint if the coalescer's pending
    /// frame deadline has elapsed. Returns whether a frame was painted.
    pub fn fire_deferred_stream_paint(&mut self) -> bool {
        let Some(deadline) = self.stream_coalescer.pending_deadline() else {
            return false;
        };
        if self.stream_coalescer.render_due(deadline) {
            self.app.suppress_paint = true;
            self.app.render();
            return true;
        }
        false
    }

    /// Like `fire_deferred_stream_paint`, but evaluates the deadline against
    /// the REAL current instant (virtual time under a paused tokio clock)
    /// instead of forcing the deadline instant — for tests that advance the
    /// clock and assert the frame interval actually gates the paint.
    pub fn poll_deferred_stream_paint(&mut self) -> bool {
        if self.stream_coalescer.pending_deadline().is_none() {
            return false;
        }
        if self
            .stream_coalescer
            .render_due(tokio::time::Instant::now())
        {
            self.app.suppress_paint = true;
            self.app.render();
            return true;
        }
        false
    }

    /// Open help through the production handler and return the rendered frame.
    pub fn show_help_frame(&mut self) -> String {
        self.app.show_help();
        self.full_frame()
    }

    /// Abort through the real abort path (targets the active session).
    pub fn abort(&mut self) -> &mut Self {
        self.app.handle_abort();
        self.capture();
        self
    }

    /// Whether keyboard focus is currently on the side panel (vs. the input).
    pub fn focus_on_panel(&self) -> bool {
        self.app.focus_region() == Focus::Panel
    }

    /// The terminal width this harness was constructed with (independent of
    /// rendered frame content — useful for assertions that must not derive
    /// width from the frame itself).
    pub fn terminal_width(&self) -> usize {
        self.width
    }

    /// The 0-based panel highlight index.
    pub fn highlight(&self) -> usize {
        self.app.panel_highlight_index()
    }

    /// The active agent's id (`None` = master).
    pub fn active_agent(&self) -> Option<String> {
        self.app.active_agent_id().map(str::to_string)
    }

    /// The full screen frame (ANSI-stripped), joined into one string.
    pub fn full_frame(&mut self) -> String {
        self.app
            .compose_frame()
            .iter()
            .map(|l| strip_ansi(l))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Append a user-visible conversation line through the real chat renderer.
    pub fn add_user_message(&mut self, text: &str) -> &mut Self {
        self.app.active_chat_mut().add_entry(ChatEntry::User {
            text: text.to_string(),
        });
        self.capture();
        self
    }

    /// Copy a visible-frame selection through the production extraction helper.
    pub fn extract_visible_selection(
        &mut self,
        start_col: u16,
        start_row: u16,
        end_col: u16,
        end_row: u16,
    ) -> String {
        let start = SelectionAnchor {
            col: start_col,
            row: start_row,
        };
        let end = SelectionAnchor {
            col: end_col,
            row: end_row,
        };
        self.app.selection = Some(TextSelection { start, end });
        self.app.compose_frame();
        self.app.selection = None;
        self.app.extract_selection(&start, &end)
    }

    /// The first visible column belonging to the main conversation body.
    pub fn body_start_col(&self) -> usize {
        let (panel_width, divider_width, _) = self.app.frame_split();
        panel_width + divider_width
    }

    /// The full screen frame WITH ANSI styling, joined into one string — for
    /// asserting style changes (e.g. the focus-highlighted divider).
    pub fn full_frame_raw(&mut self) -> String {
        self.app.compose_frame().join("\n")
    }

    /// The bottom stack (below-chat section), ANSI-stripped — for asserting on
    /// what does (or no longer does) render in the input/footer area (#820).
    pub fn bottom_stack(&mut self) -> String {
        // Render at the reduced body width the real frame uses once the panel
        // is on (#820 review), not the full terminal width.
        let width = self.app.body_width();
        self.app
            .compose_bottom(width)
            .iter()
            .map(|l| strip_ansi(l))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The main-pane (top) region of the frame — everything above the bottom
    /// stack — ANSI-stripped, for asserting on the relocated workflow bar (#820).
    pub fn main_pane(&mut self) -> String {
        // Slice the real frame at the same body width compose_frame used, so the
        // top/bottom split matches what the user sees (#820 review).
        let width = self.app.body_width();
        let bottom_len = self.app.compose_bottom(width).len();
        let frame = self.app.compose_frame();
        let top = &frame[..frame.len().saturating_sub(bottom_len)];
        top.iter()
            .map(|l| strip_ansi(l))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The left sub-agent panel region of the frame — the first `panel_width`
    /// columns of every line, ANSI-stripped — for asserting on panel content
    /// such as the read-only observer marker (#966).
    pub fn left_panel(&mut self) -> String {
        let (panel_width, _, _) = self.app.frame_split();
        self.app
            .compose_frame()
            .iter()
            .map(|l| {
                let stripped = strip_ansi(l);
                let chars: Vec<char> = stripped.chars().collect();
                let take: Vec<char> = chars.into_iter().take(panel_width).collect();
                take.into_iter().collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub async fn drain_commands(&mut self) -> Vec<String> {
        // `send_command` dispatches via a fire-and-forget spawn that may run on
        // another worker thread under load; poll (bounded) for it so sharded
        // runs aren't flaky, returning as soon as a command arrives.
        let mut out = Vec::new();
        for _ in 0..400 {
            while let Ok(line) = self.cmd_rx.try_recv() {
                out.push(line);
            }
            if !out.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
        out
    }
}

/// Lines present (normalized) in exactly one frame and absent from both
/// neighbours — a line that flashes in and out.
pub(super) fn transient_in(frames: &[Vec<String>]) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    if frames.len() < 3 {
        return out;
    }
    for i in 1..frames.len() - 1 {
        for line in &frames[i] {
            if line.trim().is_empty() {
                continue;
            }
            let key = normalize(line);
            let in_prev = frames[i - 1].iter().any(|l| normalize(l) == key);
            let in_next = frames[i + 1].iter().any(|l| normalize(l) == key);
            if !in_prev && !in_next {
                out.push((i, line.clone()));
            }
        }
    }
    out
}
