//! Layout/flicker regression tests driven by the headless render harness.
//!
//! These reproduce the multi-agent scenarios that caused the sub-agent panel
//! judder and the workflow-bar flash, and assert on layout stability — no
//! manual eyeballing of a live session required.

use super::tui_harness::*;
use crate::infrastructure::client::Event;
use crate::infrastructure::render::DiffRenderer;
use crate::interface::theme::BG_SUCCESS;
use std::sync::{Arc, Mutex};

/// A batch of sub-agents spawning and running workflows must not produce a
/// single-frame "flash" (height spike/dip) or a transient line in the below-chat
/// region. (Monotonic growth as agents appear is allowed; flicker is not.)
#[tokio::test]
async fn multi_agent_spawn_and_workflow_has_no_flash() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);

    // Spawn 5 agents one by one (each registers, then a state push arrives).
    let mut infos = Vec::new();
    for i in 1..=5 {
        let id = format!("ui-test-a{i}");
        h.event(spawn_start(&id));
        infos.push(subagent(&id, "running", Some(("active", 0, 3))));
        h.event(subagents_changed(infos.clone()));
        h.tick();
        h.tick();
    }

    // Workflows advance 0→3 across all agents.
    for done in 1..=3 {
        let infos: Vec<_> = (1..=5)
            .map(|i| {
                subagent(
                    &format!("ui-test-a{i}"),
                    "running",
                    Some(("active", done, 3)),
                )
            })
            .collect();
        h.event(subagents_changed(infos));
        h.tick();
        h.tick();
    }

    // The judder signal is a height spike/dip (a line that pops in then out).
    // Monotonic growth as agents appear is fine; content changes on a stable
    // row (Starting→Running, wf 0/3→1/3) are not flicker.
    assert!(
        h.flashes().is_empty(),
        "below-chat height spiked/dipped (flash): {:?}\nheights = {:?}\n{}",
        h.flashes(),
        h.heights(),
        h.dump()
    );
}

/// A child's forwarded `workflow_state` (including from a not-yet-registered
/// child — the "first load" race) must never render a parent workflow bar.
#[tokio::test]
async fn forwarded_child_workflow_never_renders_parent_bar() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![
        subagent("a1", "running", Some(("active", 0, 5))),
        subagent("a2", "running", Some(("active", 0, 5))),
    ]));

    // Forwarded events for known AND unknown (unregistered) children.
    for done in 1..=5 {
        h.event(forwarded_workflow("a1", done, 5));
        h.event(forwarded_workflow("a2", done, 5));
        h.event(forwarded_workflow("not-yet-registered", done, 5));
    }

    let dump = h.dump();
    // The parent workflow bar would render a percent and an issue line; neither
    // should ever appear (the per-agent rows show "wf …/…" without a percent).
    assert!(
        !dump.contains('%'),
        "a forwarded child workflow rendered a percent (parent bar leak):\n{dump}"
    );
    assert!(
        h.flashes().is_empty(),
        "forwarded child events caused a height flash: {:?}\nheights = {:?}",
        h.flashes(),
        h.heights()
    );
}

/// Sub-agent-first layout (#820): the bottom sub-agent bar — and with it the
/// per-row `awaiting` indicator — no longer renders. Agents now live in the
/// always-on left panel, so the bottom stack must never carry an await line and
/// the agent rows appear in the panel (full frame) instead.
#[tokio::test]
async fn awaiting_indicator_gone_agents_live_in_panel() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![
        subagent("worker", "running", Some(("active", 1, 3))),
        subagent("other", "running", Some(("active", 1, 3))),
    ]));

    h.event(await_start("worker"));
    let dump = h.dump();
    assert!(
        !dump.contains("awaiting"),
        "the awaiting indicator moved out of the bottom stack with the bar:\n{dump}"
    );
    // The agents themselves are listed in the always-on panel (full frame).
    let full = h.dump_full();
    assert!(
        full.contains("worker") && full.contains("other"),
        "sub-agent rows must appear in the left panel:\n{full}"
    );

    h.event(tool_end("tc-await-worker", "agent_cmd"));
    assert!(
        !h.last().contains("awaiting"),
        "no await indicator may render in the bottom stack:\n{}",
        h.last()
    );
}

/// A genuine `agent_cmd get_state` tool call must render a tool box in the chat,
/// consistent with `get_messages`/`await` (#865). The box header carries the
/// `command → agent_id` detail. (The TUI's OWN internal get_state polling flows
/// through Response events / `app_response.rs`, not this tool path, so it is
/// unaffected and stays box-free.)
#[tokio::test]
async fn agent_cmd_get_state_renders_a_tool_box() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent(
        "child",
        "running",
        Some(("active", 5, 5)),
    )]));
    h.event(Event::ToolExecutionStart {
        tool_call_id: "t1".into(),
        tool_name: "agent_cmd".into(),
        args: serde_json::json!({"command":"get_state","agent_id":"child"}),
    });
    h.event(Event::ToolExecutionEnd {
        tool_call_id: "t1".into(),
        tool_name: "agent_cmd".into(),
        result: serde_json::json!({"content":[{"type":"text","text":"{\"isStreaming\":false,\"workflow\":{\"mode\":\"complete\",\"progress\":{\"done\":5,\"total\":5,\"percent\":100}}}"}]}),
        is_error: false,
    });
    h.tick();
    let dump = h.dump_full();
    assert!(
        dump.contains("get_state → child"),
        "agent_cmd get_state should render a tool box in the chat (#865):\n{dump}"
    );
    // Pin the box BODY, not just the header: the completed box shows a green
    // success tick, proving the result actually rendered into the box (the same
    // result-preview path as `get_messages`/`await`), not an empty/pending shell.
    assert!(
        dump.contains('✓'),
        "completed get_state box must render its success result body (#865):\n{dump}"
    );
}

/// #871: control/destructive `agent_cmd` commands (abort/kill) append a chat
/// entry on the master path, just like read-only queries do — so the transcript
/// stays complete and it's clear why a sub-agent stopped.
#[tokio::test]
async fn agent_cmd_abort_and_kill_append_chat_entry_on_master_path() {
    for cmd in ["abort", "kill"] {
        let mut h = TuiHarness::new().await;
        let before = h.app_mut().master_session.chat.entry_count();
        h.event(Event::ToolExecutionStart {
            tool_call_id: "t1".into(),
            tool_name: "agent_cmd".into(),
            args: serde_json::json!({"command":cmd,"agent_id":"worker-1"}),
        });
        let after = h.app_mut().master_session.chat.entry_count();
        assert_eq!(
            after,
            before + 1,
            "agent_cmd {cmd} should append a chat entry"
        );
    }
}

/// #871: control/destructive `agent_cmd` commands (abort/kill) must render a
/// tool box in the chat, the same way `get_state`/`await` do. A frame-level
/// assertion guards against a predicate-only false positive.
#[tokio::test]
async fn agent_cmd_abort_and_kill_render_tool_boxes() {
    for cmd in ["abort", "kill"] {
        let mut h = TuiHarness::new().await;
        h.event(Event::AgentStart);
        h.event(subagents_changed(vec![subagent(
            "child",
            "running",
            Some(("active", 5, 5)),
        )]));
        h.event(Event::ToolExecutionStart {
            tool_call_id: "t1".into(),
            tool_name: "agent_cmd".into(),
            args: serde_json::json!({"command":cmd,"agent_id":"child"}),
        });
        h.event(Event::ToolExecutionEnd {
            tool_call_id: "t1".into(),
            tool_name: "agent_cmd".into(),
            result: serde_json::json!({"content":[{"type":"text","text":"ok"}]}),
            is_error: false,
        });
        h.tick();
        let dump = h.dump_full();
        assert!(
            dump.contains(&format!("{cmd} → child")),
            "agent_cmd {cmd} should render a tool box in the chat (#871):\n{dump}"
        );
        assert!(
            dump.contains('✓'),
            "completed {cmd} box must render its success body (#871):\n{dump}"
        );
    }
}

/// #871: control/destructive `agent_cmd` commands routed into a SELECTED
/// sub-agent's direct stream must render the same tool box in that sub-agent's
/// chat, mirroring the master path.
#[tokio::test]
async fn agent_cmd_abort_and_kill_render_tool_boxes_in_subagent_view() {
    for cmd in ["abort", "kill"] {
        let mut h = TuiHarness::new().await;
        h.event(Event::AgentStart);
        h.event(subagents_changed(vec![subagent(
            "child",
            "running",
            Some(("active", 5, 5)),
        )]));
        h.select(Some("child"));
        h.route(
            "child",
            Event::ToolExecutionStart {
                tool_call_id: "s1".into(),
                tool_name: "agent_cmd".into(),
                args: serde_json::json!({"command":cmd,"agent_id":"grandchild"}),
            },
        );
        h.route(
            "child",
            Event::ToolExecutionEnd {
                tool_call_id: "s1".into(),
                tool_name: "agent_cmd".into(),
                result: serde_json::json!({"content":[{"type":"text","text":"ok"}]}),
                is_error: false,
            },
        );
        h.tick();
        let dump = h.dump_full();
        assert!(
            dump.contains(&format!("{cmd} → grandchild")),
            "agent_cmd {cmd} must render a tool box in the sub-agent view too (#871):\n{dump}"
        );
    }
}

/// The sub-agent view path (#865 acceptance: BOTH views). A genuine
/// `agent_cmd get_state` routed into a SELECTED sub-agent's direct stream must
/// render the same tool box in that sub-agent's chat, mirroring the master path.
#[tokio::test]
async fn agent_cmd_get_state_renders_a_tool_box_in_subagent_view() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent(
        "child",
        "running",
        Some(("active", 5, 5)),
    )]));
    h.select(Some("child"));
    h.route(
        "child",
        Event::ToolExecutionStart {
            tool_call_id: "s1".into(),
            tool_name: "agent_cmd".into(),
            args: serde_json::json!({"command":"get_state","agent_id":"grandchild"}),
        },
    );
    h.route(
        "child",
        Event::ToolExecutionEnd {
            tool_call_id: "s1".into(),
            tool_name: "agent_cmd".into(),
            result: serde_json::json!({"content":[{"type":"text","text":"ok"}]}),
            is_error: false,
        },
    );
    h.tick();
    let dump = h.dump_full();
    assert!(
        dump.contains("get_state → grandchild"),
        "agent_cmd get_state must render a tool box in the sub-agent view too (#865):\n{dump}"
    );
    assert!(
        dump.contains('✓'),
        "completed get_state box must render its success body in the sub-agent view (#865):\n{dump}"
    );
}

/// The real-log judder: workflow sub-agents fire notifications, so the parent
/// does many short runs, each creating/dropping the spinner. With sub-agents
/// present that 0↔1 spinner toggle must NOT reflow the chat (its line is
/// reserved), or the panel size oscillates (6↔7 / 11↔12) on every run.
#[tokio::test]
async fn spinner_blink_with_subagents_does_not_reflow() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(
        (1..=5)
            .map(|i| subagent(&format!("a{i}"), "running", Some(("active", 1, 3))))
            .collect(),
    ));
    // Parent runs repeatedly (one per subagent notification): spinner blinks.
    for _ in 0..5 {
        h.event(Event::AgentEnd { messages: vec![] }); // spinner off
        h.event(Event::AgentStart); // spinner on
    }
    assert!(
        h.flashes().is_empty(),
        "spinner blink reflowed the chat (height flash): {:?}\nheights = {:?}",
        h.flashes(),
        h.heights()
    );
}

/// End-to-end guard for the parse-error leak: forwarded child workflow_state
/// arrives as a raw wire line WITHOUT `steps`. It must deserialize (so the
/// client never drops it / prints it over the TUI), and it must not render
/// into the frame (it's a child's, ignored by agent_id).
#[tokio::test]
async fn forwarded_canonical_wire_line_parses_and_does_not_leak() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent(
        "tui-ui-test-3",
        "running",
        Some(("active", 3, 5)),
    )]));
    // The exact shape the monitor forwards (canonical, no `steps`).
    h.event_line(
        r#"{"type":"workflow_state","agent_id":"tui-ui-test-3","parent_id":null,"mode":"active","progress":{"done":3,"total":5,"percent":60}}"#,
    );
    assert!(
        !h.dump_full().contains("percent"),
        "forwarded workflow_state leaked a percent into the frame:\n{}",
        h.dump_full()
    );
}

/// A `get_subagents` poll carries no workflow snapshot. Sub-agent-first (#820):
/// the per-row `n/n` workflow snapshot no longer renders (the bottom bar is
/// gone), but the agent row itself must survive a workflowless poll — it stays
/// listed in the always-on left panel rather than vanishing.
#[tokio::test]
async fn subagent_row_survives_workflowless_poll() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent(
        "a1",
        "running",
        Some(("active", 2, 3)),
    )]));
    assert!(
        h.dump_full().contains("a1"),
        "agent row should show in the panel:\n{}",
        h.dump_full()
    );
    // get_subagents response shape: status only, no workflow.
    h.event(subagents_changed(vec![subagent("a1", "running", None)]));
    assert!(
        h.dump_full().contains("a1"),
        "agent row must persist through a workflowless poll:\n{}",
        h.dump_full()
    );
}

/// While the parent is idle but a child is still active, the reserved spinner
/// slot shows an animated "N working" indicator (not a blank) — the "missing
/// working spinner" report.
#[tokio::test]
async fn idle_parent_shows_subagent_activity() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent(
        "a1",
        "running",
        Some(("active", 2, 3)),
    )]));
    h.event(Event::AgentEnd { messages: vec![] }); // parent idle, a1 still running
    assert!(
        h.last().contains("subagent working"),
        "idle parent with an active child should show an activity line:\n{}",
        h.last()
    );
}

#[tokio::test]
async fn harness_gc_removes_exited_subagents_and_captures() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent("done", "exited", None)]));
    let before = h.heights().len();
    h.gc();
    assert_eq!(h.heights().len(), before + 1);
}

#[tokio::test]
async fn transient_detection_normalizes_spinner_and_digits() {
    let frames = vec![
        vec!["worker 1 ⠁".to_string()],
        vec!["flash 123".to_string(), "worker 2 ⠂".to_string()],
        vec!["worker 3 ⠄".to_string()],
    ];
    let transients = transient_in(&frames);
    assert_eq!(transients, vec![(1, "flash 123".to_string())]);
}

#[tokio::test]
async fn event_line_invalid_json_panics_with_context() {
    let mut h = TuiHarness::new().await;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.event_line("not-json");
    }));
    assert!(result.is_err());
}

// ── #884: differential renderer desync at full height ──────────────────
//
// Drive the REAL render path (full-height frames) through the actual
// `DiffRenderer`, with a `BG_SUCCESS` tool box on screen and a churning
// bottom (spinner ticks). A tiny terminal emulator replays the emitted bytes
// so we can assert the user-visible result: exactly one activity line, no row
// drift / ghost spinner lines, and no `\x1b[2K` erase performed while a tool
// box background is still active (the green panel bleed).

/// Minimal VT emulator: replays the renderer's byte stream onto a fixed grid so
/// tests see what the terminal would actually show. Models the bottom-row
/// scroll on `\n` that the differential renderer's relative stepping could
/// trigger (the root cause of the ghost/jitter), plus DEC auto-wrap (DECAWM):
/// a full-width glyph on the bottom row scrolls the viewport iff auto-wrap was
/// left enabled — so the renderer's `?7l` guard is actually exercised.
struct Vt {
    w: usize,
    h: usize,
    grid: Vec<Vec<char>>,
    row: usize,
    col: usize,
    /// DEC auto-wrap mode (DECAWM). Default on; toggled by `?7h`/`?7l`.
    wrap: bool,
    /// Deferred-wrap flag: a glyph written in the last column arms a wrap that
    /// only fires (when `wrap` is on) on the NEXT printable glyph — modelling
    /// real terminals, so a full-width bottom line scrolls the viewport iff
    /// auto-wrap was left enabled.
    pending_wrap: bool,
}

impl Vt {
    fn new(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            grid: vec![vec![' '; w]; h],
            row: 0,
            col: 0,
            wrap: true,
            pending_wrap: false,
        }
    }

    fn scroll_up(&mut self) {
        self.grid.remove(0);
        self.grid.push(vec![' '; self.w]);
    }

    fn linefeed(&mut self) {
        if self.row + 1 >= self.h {
            self.scroll_up();
        } else {
            self.row += 1;
        }
    }

    fn apply(&mut self, s: &str) {
        let chars: Vec<char> = s.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            match c {
                '\x1b' if chars.get(i + 1) == Some(&'[') => {
                    i += 2;
                    let mut params = String::new();
                    let mut private = false;
                    while i < chars.len() {
                        let p = chars[i];
                        if p == '?' {
                            private = true;
                            i += 1;
                        } else if p.is_ascii_digit() || p == ';' {
                            params.push(p);
                            i += 1;
                        } else {
                            break;
                        }
                    }
                    let final_byte = chars.get(i).copied().unwrap_or(' ');
                    i += 1;
                    if private {
                        // ?2026h/l (sync) — irrelevant to the grid; ?7l/h toggle
                        // auto-wrap, which DOES change scroll behaviour.
                        if params == "7" {
                            self.wrap = final_byte == 'h';
                            self.pending_wrap = false;
                        }
                        continue;
                    }
                    let nums: Vec<usize> =
                        params.split(';').filter_map(|p| p.parse().ok()).collect();
                    // Any explicit cursor positioning cancels a deferred wrap.
                    self.pending_wrap = false;
                    match final_byte {
                        'H' => {
                            let r = nums.first().copied().unwrap_or(1).max(1) - 1;
                            let col = nums.get(1).copied().unwrap_or(1).max(1) - 1;
                            self.row = r.min(self.h - 1);
                            self.col = col;
                        }
                        'A' => {
                            let n = nums.first().copied().unwrap_or(1);
                            self.row = self.row.saturating_sub(n);
                        }
                        'B' => {
                            let n = nums.first().copied().unwrap_or(1);
                            self.row = (self.row + n).min(self.h - 1);
                        }
                        'K' => {
                            // 2K (or K) — erase line.
                            for cell in self.grid[self.row].iter_mut() {
                                *cell = ' ';
                            }
                        }
                        'J' => {
                            for r in self.grid.iter_mut() {
                                for cell in r.iter_mut() {
                                    *cell = ' ';
                                }
                            }
                        }
                        _ => {} // SGR ('m') etc — no grid effect
                    }
                }
                '\r' => {
                    self.col = 0;
                    self.pending_wrap = false;
                    i += 1;
                }
                '\n' => {
                    self.linefeed();
                    self.pending_wrap = false;
                    i += 1;
                }
                _ => {
                    // A deferred wrap (armed by a glyph in the last column) fires
                    // here iff auto-wrap is on — scrolling at the bottom row.
                    if self.wrap && self.pending_wrap {
                        self.linefeed();
                        self.col = 0;
                    }
                    self.pending_wrap = false;
                    if self.col < self.w {
                        self.grid[self.row][self.col] = c;
                    }
                    if self.col + 1 >= self.w {
                        // Last column: arm a deferred wrap rather than advancing.
                        self.pending_wrap = true;
                    } else {
                        self.col += 1;
                    }
                    i += 1;
                }
            }
        }
    }

    fn rows_stripped(&self) -> Vec<String> {
        self.grid
            .iter()
            .map(|r| r.iter().collect::<String>().trim_end().to_string())
            .collect()
    }
}

/// `true` if anywhere in the byte stream an `ERASE_LINE` (`\x1b[2K`) is emitted
/// while a `BG_SUCCESS` background is still the active SGR state (the green
/// panel-bleed defect): a reset (`\x1b[0m`) clears the active background.
fn erases_under_active_bg(stream: &str) -> bool {
    let b = stream.as_bytes();
    let erase = b"\x1b[2K";
    let reset = b"\x1b[0m";
    let bg = BG_SUCCESS.as_bytes();
    let mut bg_active = false;
    let mut i = 0;
    while i < b.len() {
        if b[i..].starts_with(erase) {
            if bg_active {
                return true;
            }
            i += erase.len();
        } else if b[i..].starts_with(bg) {
            bg_active = true;
            i += bg.len();
        } else if b[i..].starts_with(reset) {
            bg_active = false;
            i += reset.len();
        } else {
            i += 1;
        }
    }
    false
}

#[tokio::test]
async fn full_height_multi_agent_renderer_no_desync_or_bleed() {
    const W: usize = 120;
    const H: usize = 40;
    let mut h = TuiHarness::sized(W, H).await;
    h.event(Event::AgentStart);

    // Fill the screen with a fleet of running sub-agents so the composed frame
    // reaches full terminal height and the panel/chat are busy.
    let mut infos = Vec::new();
    for i in 1..=8 {
        let id = format!("agent-{i}");
        h.event(spawn_start(&id));
        infos.push(subagent(&id, "running", Some(("active", 1, 3))));
    }
    h.event(subagents_changed(infos));
    // End the spawn tool calls so the master's own spinner clears: the parent is
    // now idle while its children run → the below-chat reserved slot shows the
    // animated "N subagents working..." activity line (the line that ghosts).
    for i in 1..=8 {
        h.event(tool_end(&format!("tc-spawn-agent-{i}"), "spawn"));
    }
    h.event(Event::AgentEnd {
        messages: Vec::new(),
    });

    // Capture-only renderer fed the REAL composed frames. A green BG_SUCCESS
    // tool box is spliced onto a chat row so a success box is on-screen exactly
    // while the churning bottom rows are erased/repainted.
    let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    struct Shared(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Shared {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut renderer = DiffRenderer::new(Shared(buf.clone()));

    let box_row = 4usize;
    let green_box = format!("{BG_SUCCESS}  cargo build  ✓  \x1b[0m");

    let frame_with_box = |app: &mut crate::interface::app::App| -> Vec<String> {
        let mut f = app.compose_frame();
        while f.len() < H {
            f.push(String::new());
        }
        f[box_row] = green_box.clone();
        f
    };

    // First frame establishes the baseline (full render).
    let first = frame_with_box(h.app_mut());
    renderer.render(&first, W).unwrap();
    // Byte offset where the full render ends — everything after is diff output.
    let diff_start = buf.lock().unwrap().len();

    // Churn: spinner ticks change the bottom activity row repeatedly while the
    // green box stays put. Each tick is a fresh diff_render against full height.
    let mut last_frame = first;
    for _ in 0..12 {
        h.tick();
        let f = frame_with_box(h.app_mut());
        renderer.render(&f, W).unwrap();
        last_frame = f;
    }

    let raw = buf.lock().unwrap().clone();
    let stream = String::from_utf8_lossy(&raw).to_string();
    let diff_only = String::from_utf8_lossy(&raw[diff_start..]).to_string();

    // (b0) Diff renders must move vertically with absolute addressing only — a
    // bare `\r\n` step on the bottom row scrolls the viewport and desyncs the
    // renderer's row model (the ghost-line / jitter root cause).
    assert!(
        !diff_only.contains("\r\n"),
        "diff renders used \\r\\n vertical stepping (can scroll the viewport)"
    );
    assert!(
        diff_only.contains(";1H"),
        "diff renders must use absolute cursor addressing"
    );

    // (b0b) Each diff paint must bracket itself with auto-wrap OFF→ON so a
    // full-width line on the bottom row can never auto-scroll the viewport (the
    // belt to absolute-addressing's braces). The VT above models DECAWM, so the
    // (c) drift check stays honest if this guard is ever dropped.
    let off = diff_only.find("\x1b[?7l");
    let on = diff_only.find("\x1b[?7h");
    assert!(
        off.is_some() && on.is_some() && off < on,
        "diff paint must disable auto-wrap (\\x1b[?7l) before re-enabling it"
    );

    // (b) Sanity guard: no erase inherits a tool-box background anywhere in the
    // stream. NOTE: defect #2 (the green panel bleed) is genuinely reproduced by
    // the `diff_render_resets_sgr_before_erasing` UNIT test (which is RED against
    // the old renderer). Here the composed green box self-resets and the diffs
    // only touch bottom rows, so this assertion is a non-regression guard, not a
    // standalone reproduction of the defect.
    assert!(
        !erases_under_active_bg(&stream),
        "an ERASE_LINE fired while BG_SUCCESS was active (green panel bleed)"
    );

    // Replay the whole byte stream onto a virtual terminal.
    let mut vt = Vt::new(W, H);
    vt.apply(&stream);
    let screen = vt.rows_stripped();
    let expected: Vec<String> = last_frame.iter().map(|l| strip_ansi_line(l)).collect();

    // (a) Exactly one "N working" activity line on screen — no ghost/duplicate.
    let working_rows = screen.iter().filter(|r| r.contains("working...")).count();
    assert_eq!(
        working_rows,
        1,
        "expected exactly one activity line, got {working_rows}\nscreen:\n{}",
        screen.join("\n")
    );

    // (c) No row drift: the replayed terminal matches the last composed frame.
    assert_eq!(
        screen, expected,
        "rendered terminal drifted from the composed frame (ghosting/jitter)"
    );
}

fn strip_ansi_line(s: &str) -> String {
    crate::interface::ansi::strip_ansi(s).trim_end().to_string()
}
