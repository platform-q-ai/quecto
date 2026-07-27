//! Steps for `tui_chat_spacing.feature` (#480).
//!
//! Drives the REAL frame composition (`App::compose_frame`) through the headless
//! render harness at a fixed terminal size and asserts on the observable layout:
//! a short chat keeps at least `MIN_CHAT_GAP` blank lines above the editor
//! border, and a chat that overflows auto-scrolls to the latest content.

use crate::{TuiParityHarness, TuiWorld};
use cucumber::{given, then, when};
use quecto_tui::shell::app::tui_harness::TuiHarness;

const ROWS: usize = 30;

fn with_harness<R>(world: &mut TuiWorld, f: impl FnOnce(&mut TuiHarness) -> R) -> R {
    if world.tui_parity_rt.is_none() {
        world.tui_parity_rt = Some(tokio::runtime::Runtime::new().expect("tokio runtime"));
    }
    if world.tui_parity.is_none() {
        let rt = world.tui_parity_rt.as_ref().expect("runtime");
        let h = rt.block_on(async { TuiHarness::sized(120, ROWS).await });
        world.tui_parity = Some(TuiParityHarness(h));
    }
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("runtime")
        .handle()
        .clone();
    let _guard = handle.enter();
    f(&mut world.tui_parity.as_mut().expect("TUI harness").0)
}

/// The main body of a rendered frame line, with the persistent left sub-agent
/// panel and its `│` divider stripped off, so blank-gap detection sees the true
/// body content rather than the panel's column fill.
fn body(line: &str) -> &str {
    line.rsplit('│').next().unwrap_or(line)
}

/// Split the full frame into body-only lines (panel prefix removed).
fn body_lines(frame: &str) -> Vec<String> {
    frame.split('\n').map(|l| body(l).to_string()).collect()
}

/// Index of the editor's top border line (the only body line carrying the ` > `
/// prompt indicator inside a run of `─` box characters — distinct from chat
/// `> ` message prefixes, which carry no `─` glyphs).
fn editor_border_index(lines: &[String]) -> usize {
    lines
        .iter()
        .position(|l| l.contains(" > ") && l.chars().filter(|c| *c == '─').count() >= 10)
        .expect("the editor top border should be present in the frame")
}

fn blank_lines_above(lines: &[String], idx: usize) -> usize {
    lines[..idx]
        .iter()
        .rev()
        .take_while(|l| l.trim().is_empty())
        .count()
}

// ── Given ──────────────────────────────────────────────────────────────────

#[given("the chat has 5 lines of content")]
fn chat_5_lines(world: &mut TuiWorld) {
    with_harness(world, |h| {
        for i in 1..=5 {
            h.add_user_message(&format!("line {i}"));
        }
    });
}

#[given("the terminal has 30 rows")]
fn terminal_30_rows(world: &mut TuiWorld) {
    let frame = with_harness(world, |h| h.full_frame());
    assert_eq!(
        frame.split('\n').count(),
        ROWS,
        "the composed frame should be exactly 30 rows tall"
    );
}

#[given("the chat fills the entire available space")]
fn chat_fills_space(world: &mut TuiWorld) {
    with_harness(world, |h| {
        for i in 0..60 {
            h.add_user_message(&format!("msg-{i}"));
        }
    });
}

// ── When ───────────────────────────────────────────────────────────────────

#[when("the screen renders")]
fn screen_renders(world: &mut TuiWorld) {
    // Render once through the real compose path; assertions re-render as needed.
    let _ = with_harness(world, |h| h.full_frame());
}

// ── Then ───────────────────────────────────────────────────────────────────

#[then("at least 3 blank lines should appear between chat and editor border")]
fn at_least_3_blank_lines(world: &mut TuiWorld) {
    let frame = with_harness(world, |h| h.full_frame());
    let lines = body_lines(&frame);
    let border = editor_border_index(&lines);
    let blanks = blank_lines_above(&lines, border);
    assert!(
        blanks >= 3,
        "expected at least 3 blank lines between chat and editor border, got {blanks}\n{frame}"
    );
}

#[then("chat is scrolled to show the latest content")]
fn chat_scrolled_to_latest(world: &mut TuiWorld) {
    let frame = with_harness(world, |h| h.full_frame());
    assert!(
        frame.contains("msg-59"),
        "the latest message should be visible (auto-scrolled), frame:\n{frame}"
    );
    assert!(
        !frame.contains("msg-0\n") && !frame.contains("msg-0 "),
        "the oldest message should have scrolled off the top, frame:\n{frame}"
    );
}

#[then("spacing may be reduced to fit content")]
fn spacing_reduced(world: &mut TuiWorld) {
    let frame = with_harness(world, |h| h.full_frame());
    let lines = body_lines(&frame);
    let border = editor_border_index(&lines);
    let blanks = blank_lines_above(&lines, border);
    let chat_lines = lines.iter().filter(|l| l.contains("msg-")).count();
    // Content took priority over spacing: the overflowing chat occupies more of
    // the reserved area than the blank gap that a short chat would leave.
    assert!(
        blanks < chat_lines,
        "chat content ({chat_lines} lines) should dominate the reduced gap ({blanks} blanks)\n{frame}"
    );
}
