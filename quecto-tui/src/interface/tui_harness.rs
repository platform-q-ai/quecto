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
use crate::infrastructure::client::{Client, Event, SubagentInfoEvent, SubagentWorkflow};
use crate::infrastructure::terminal::Terminal;
use crate::interface::components::chat::ChatEntry;
use crate::interface::components::notification::NotifyLevel;
use crate::interface::components::spinner::Spinner;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

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

    /// Apply a branch change through the production footer update path.
    pub fn apply_branch(&mut self, branch: Option<String>) -> bool {
        self.app.apply_git_branch(branch)
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
        for _ in 0..5 {
            tokio::task::yield_now().await;
        }
        let mut out = Vec::new();
        while let Ok(line) = self.cmd_rx.try_recv() {
            out.push(line);
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

/// Normalize a rendered line for transient detection: drop braille spinner
/// frames and collapse digit runs (elapsed seconds) so animation ticks don't
/// register as appearing/disappearing content.
fn normalize(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut last_was_digit = false;
    for c in line.chars() {
        if ('\u{2800}'..='\u{28ff}').contains(&c) {
            continue; // braille spinner frame
        }
        if c.is_ascii_digit() {
            if !last_was_digit {
                out.push('#');
            }
            last_was_digit = true;
            continue;
        }
        last_was_digit = false;
        out.push(c);
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ── Scenario event builders ───────────────────────────────────────────

/// A `SubagentInfoEvent`, optionally carrying a workflow snapshot `(mode, done, total)`.
pub fn subagent(id: &str, status: &str, wf: Option<(&str, u32, u32)>) -> SubagentInfoEvent {
    subagent_with_socket(id, status, wf, None)
}

/// Bind a real, drained Unix socket for a sub-agent and return its path. The
/// listener task accepts one connection and drains its lines, so a TUI
/// `connect-on-select` to this path succeeds and the per-child command channel
/// stays live (its receiver is NOT dropped) — letting routing tests exercise
/// the real `try_send` delivery path rather than the older-kernel `None` case.
pub fn spawn_subagent_socket(id: &str) -> std::path::PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "quecto-tui-harness-sub-{}-{}-{}",
        std::process::id(),
        id,
        n
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let socket_path = dir.join("agent.sock");
    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
    tokio::spawn(async move {
        // Loop accepting so reselecting the same agent (teardown + reconnect)
        // still finds a live listener.
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut lines = BufReader::new(stream).lines();
                while let Ok(Some(_line)) = lines.next_line().await {}
            });
        }
    });
    socket_path
}

/// A `SubagentInfoEvent` carrying an explicit `socket_path` (live connection).
pub fn subagent_with_socket(
    id: &str,
    status: &str,
    wf: Option<(&str, u32, u32)>,
    socket_path: Option<std::path::PathBuf>,
) -> SubagentInfoEvent {
    SubagentInfoEvent {
        agent_id: id.to_string(),
        status: status.to_string(),
        last_tool: None,
        last_error: None,
        pid: 0,
        socket_path: socket_path.map(|p| p.to_string_lossy().into_owned()),
        parent_id: None,
        workflow: wf.map(|(mode, d, t)| SubagentWorkflow {
            mode: mode.to_string(),
            steps_completed: d,
            steps_total: t,
        }),
        read_only: false,
    }
}

/// A read-only sub-agent event (`read_only: true`) for observer-marker tests
/// (#966). Otherwise identical to `subagent_with_socket`.
pub fn subagent_readonly(
    id: &str,
    status: &str,
    wf: Option<(&str, u32, u32)>,
    socket_path: Option<std::path::PathBuf>,
) -> SubagentInfoEvent {
    let mut ev = subagent_with_socket(id, status, wf, socket_path);
    ev.read_only = true;
    ev
}

/// `get_subagents`-style push of the full sub-agent list.
pub fn subagents_changed(list: Vec<SubagentInfoEvent>) -> Event {
    Event::SubagentStateChanged { subagents: list }
}

/// A `spawn` tool starting (registers the child locally as "starting").
pub fn spawn_start(id: &str) -> Event {
    Event::ToolExecutionStart {
        tool_call_id: format!("tc-spawn-{id}"),
        tool_name: "spawn".to_string(),
        args: serde_json::json!({ "agent_id": id }),
    }
}

/// An `agent_cmd await` tool starting on `id` (marks the row "awaiting").
pub fn await_start(id: &str) -> Event {
    Event::ToolExecutionStart {
        tool_call_id: format!("tc-await-{id}"),
        tool_name: "agent_cmd".to_string(),
        args: serde_json::json!({ "command": "await", "agent_id": id }),
    }
}

/// A tool finishing (clears the awaiting marker / spinner message).
pub fn tool_end(call_id: &str, tool: &str) -> Event {
    Event::ToolExecutionEnd {
        tool_call_id: call_id.to_string(),
        tool_name: tool.to_string(),
        result: serde_json::json!({ "content": [{ "type": "text", "text": "ok" }] }),
        is_error: false,
    }
}

/// A forwarded child `workflow_state` event (carries `agent_id` — must NOT
/// touch the parent's workflow bar).
pub fn forwarded_workflow(agent_id: &str, done: u32, total: u32) -> Event {
    Event::WorkflowState {
        agent_id: Some(agent_id.to_string()),
        steps: Vec::new(),
        progress: serde_json::json!({ "done": done, "total": total, "percent": done * 100 / total.max(1) }),
        active_issue: Some(serde_json::json!({ "number": 7, "title": "child" })),
        mode: Some("active".to_string()),
        active_template: None,
        available_templates: None,
    }
}
