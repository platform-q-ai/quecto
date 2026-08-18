//! Step definitions for `tui_multi_tab_round2.feature` (#1466 round-2 fix
//! pass, PR #1485 second field-testing round).
//!
//! Reuses the polish-suite harness plumbing (`a TUI with a second background
//! tab` lives in `tui_multi_tab_polish_steps.rs`). Sub-agent scenarios drive
//! the REAL feed-attach path (live Unix sockets), so they run inside the
//! world's runtime context.

use crate::TuiWorld;
use cucumber::{given, then, when};
use quecto_tui::components::ansi::strip_ansi;
use quecto_tui::shell::app::tui_harness::TuiHarness;

const RESTORED_MSG: &str = "hello restored agent";

/// Like the polish suite's `with_harness`, but entering the runtime handle so
/// harness drivers may spawn feed tasks (live-socket sub-agent scenarios).
fn with_harness<R>(world: &mut TuiWorld, f: impl FnOnce(&mut TuiHarness) -> R) -> R {
    if world.tui_parity_rt.is_none() {
        world.tui_parity_rt = Some(tokio::runtime::Runtime::new().expect("tokio runtime"));
    }
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("runtime")
        .handle()
        .clone();
    if world.tui_parity.is_none() {
        let h = handle.block_on(async { TuiHarness::new().await });
        world.tui_parity = Some(crate::TuiParityHarness(h));
    }
    let _guard = handle.enter();
    f(&mut world.tui_parity.as_mut().expect("TUI harness").0)
}

/// The frame's `index`th line with ANSI stripped.
fn frame_line(world: &mut TuiWorld, index: usize) -> String {
    let lines = with_harness(world, |h| h.frame_lines());
    strip_ansi(
        lines
            .get(index)
            .unwrap_or_else(|| panic!("frame has no line {index}; height={}", lines.len())),
    )
}

/// A frame line's BODY segment (past the left-panel divider, if any).
fn body_of(line: &str) -> &str {
    line.rsplit('│').next().unwrap_or(line)
}

// ── Shared context ───────────────────────────────────────────────────────

#[given("a headless TUI")]
fn given_headless_tui(world: &mut TuiWorld) {
    with_harness(world, |_| {});
}

#[when("the frame renders")]
fn when_frame_renders(world: &mut TuiWorld) {
    world.stdout = with_harness(world, |h| h.frame_lines()).join("\n");
}

// ── Item 1: version header line → blank spacer ───────────────────────────

#[then("no frame line contains the version header text")]
fn then_no_version_header(world: &mut TuiWorld) {
    assert!(
        !world.stdout.contains("quecto-tui v"),
        "the version/help header line must not render anywhere in the frame"
    );
}

#[then("the first frame line is a blank spacer")]
fn then_first_line_blank(world: &mut TuiWorld) {
    let first = frame_line(world, 0);
    assert_eq!(
        body_of(&first).trim(),
        "",
        "the first frame line must be a blank spacer; first={first:?}"
    );
}

#[then("the second frame line is a blank spacer")]
fn then_second_line_blank(world: &mut TuiWorld) {
    let second = frame_line(world, 1);
    assert_eq!(
        body_of(&second).trim(),
        "",
        "the line after the tab bar must be a blank spacer; second={second:?}"
    );
}

#[then("the first frame line is the tab bar")]
fn then_first_line_tab_bar(world: &mut TuiWorld) {
    let first = frame_line(world, 0);
    assert!(
        first.contains(" 1 ") && first.contains(" 2 "),
        "with 2+ tabs the tab bar must be the first frame line; first={first:?}"
    );
}

#[then("the frame height equals the terminal height")]
fn then_frame_height(world: &mut TuiWorld) {
    let (frame_height, term_height) =
        with_harness(world, |h| (h.frame_lines().len(), h.terminal_height()));
    assert_eq!(
        frame_height, term_height,
        "the frame must keep the full terminal height after the header swap"
    );
}

// ── Item 2: Ctrl+N new-tab chord (Ctrl+T is the tool-policy selector) ────

#[when("the user presses Ctrl+N")]
fn when_ctrl_n(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.press_raw(b"\x0e");
    });
}

#[when("the user presses Ctrl+T")]
fn when_ctrl_t(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.press_raw(b"\x14");
    });
}

#[then("a second tab is open")]
fn then_second_tab_open(world: &mut TuiWorld) {
    with_harness(world, |h| {
        assert_eq!(h.tab_count(), 2, "Ctrl+N must open a second tab");
    });
}

#[then("the new tab is the active tab")]
fn then_new_tab_active(world: &mut TuiWorld) {
    with_harness(world, |h| {
        assert_eq!(
            h.active_tab_index(),
            1,
            "the freshly opened tab must take focus"
        );
    });
}

#[then("still only one tab is open")]
fn then_still_one_tab(world: &mut TuiWorld) {
    with_harness(world, |h| {
        assert_eq!(h.tab_count(), 1, "the chord must not open a tab");
    });
}

#[then("the tool policy selector is open")]
fn then_tool_policy_open(world: &mut TuiWorld) {
    with_harness(world, |h| {
        assert!(
            h.tool_policy_selector_requested(),
            "Ctrl+T must keep requesting the tool-policy catalogue"
        );
    });
}

#[when("the user runs /hotkeys")]
fn when_runs_hotkeys(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.submit("/hotkeys");
    });
    world.stdout = with_harness(world, |h| h.last_status_line().unwrap_or_default());
}

#[then("the help text lists Ctrl+N as the new-tab chord")]
fn then_help_lists_ctrl_n(world: &mut TuiWorld) {
    let line = world
        .stdout
        .lines()
        .find(|l| l.contains("Ctrl+N ") && !l.contains("Ctrl+Shift"))
        .unwrap_or("")
        .to_lowercase();
    assert!(
        line.contains("new tab") || line.contains("open a new tab"),
        "/hotkeys must document plain Ctrl+N as the new-tab chord; help={:?}",
        world.stdout
    );
}

// ── Item 3: user sends to restored sub-agents ────────────────────────────

#[given("a running sub-agent restored from a resumed workspace is focused")]
fn given_restored_running_subagent(world: &mut TuiWorld) {
    with_harness(world, |h| h.focus_restored_running_subagent("w1"));
}

#[given("a reachable sub-agent still marked detached is focused")]
fn given_detached_reachable_subagent(world: &mut TuiWorld) {
    with_harness(world, |h| h.focus_detached_reachable_subagent("w1"));
}

// The dead-side Then ("a delivery failure naming the sub-agent is visibly
// surfaced") is bound in `tui_multi_tab_fix_pass_steps.rs` and names "w1".
#[given("a dead restored sub-agent is focused")]
fn given_dead_subagent(world: &mut TuiWorld) {
    with_harness(world, |h| h.focus_dead_subagent("w1"));
}

#[when("the user submits a message to it")]
fn when_submits_message(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.submit(RESTORED_MSG);
    });
}

#[then("the user entry appears in the sub-agent transcript")]
fn then_user_entry_in_transcript(world: &mut TuiWorld) {
    with_harness(world, |h| {
        assert!(
            h.active_user_entries().iter().any(|t| t == RESTORED_MSG),
            "the user message must land in the focused sub-agent's transcript"
        );
    });
}

#[then("no delivery-failure error is surfaced")]
fn then_no_delivery_failure(world: &mut TuiWorld) {
    with_harness(world, |h| {
        let status = h.last_status_line().unwrap_or_default();
        let note = h.last_notification().unwrap_or_default();
        assert!(
            !status.contains("not delivered") && !note.contains("not delivered"),
            "no delivery failure may surface for a reachable sub-agent; \
             status={status:?}, notification={note:?}"
        );
    });
}
