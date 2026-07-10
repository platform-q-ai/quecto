//! #884 differential-renderer regression test, split out of
//! `tui_harness_tests.rs` to keep both files within the 750-line cap.
//!
//! Drives the REAL render path (full-height frames) through the actual
//! `DiffRenderer` via a tiny VT emulator, asserting no desync / ghosting and
//! no green-panel background bleed.

use super::tui_harness::*;
use crate::infrastructure::client::Event;
use crate::infrastructure::render::DiffRenderer;
use crate::interface::theme::BG_SUCCESS;
use std::sync::{Arc, Mutex};

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
        message_refs: vec![],
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
