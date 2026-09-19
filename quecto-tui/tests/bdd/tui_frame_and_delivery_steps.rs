//! Step definitions for the single-session scenarios that outlived the
//! multi-tab features (#2044): the frame spacer/height pair in
//! `tui_foundation.feature`, the Ctrl+T guard in
//! `tui_tool_policy_modal.feature` and the sub-agent delivery trio in
//! `tui_subagent_session_parity.feature`. Sub-agent scenarios drive the REAL
//! feed-attach path (live Unix sockets), so they run inside the world's
//! runtime context.

use crate::TuiWorld;
use cucumber::{given, then, when};
use quecto_tui::components::ansi::strip_ansi;
use quecto_tui::shell::app::tui_harness::TuiHarness;

const RESTORED_MSG: &str = "hello restored agent";

/// Lazily builds the harness, entering the runtime handle so harness drivers may spawn feed tasks (live-socket sub-agent scenarios).
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

// ── Frame: blank spacer, full height ─────────────────────────────────────

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

#[then("the frame height equals the terminal height")]
fn then_frame_height(world: &mut TuiWorld) {
    let (frame_height, term_height) =
        with_harness(world, |h| (h.frame_lines().len(), h.terminal_height()));
    assert_eq!(
        frame_height, term_height,
        "the frame must keep the full terminal height after the header swap"
    );
}

// ── Ctrl+T opens the tool-policy selector ────────────────────────────────

#[when("the user presses Ctrl+T")]
fn when_ctrl_t(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.press_raw(b"\x14");
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

// ── User sends to restored sub-agents ────────────────────────────────────

#[given("a running sub-agent restored from a resumed session is focused")]
fn given_restored_running_subagent(world: &mut TuiWorld) {
    with_harness(world, |h| h.focus_restored_running_subagent("w1"));
}

#[given("a reachable sub-agent still marked detached is focused")]
fn given_detached_reachable_subagent(world: &mut TuiWorld) {
    with_harness(world, |h| h.focus_detached_reachable_subagent("w1"));
}

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

#[then("a delivery failure naming the sub-agent is visibly surfaced")]
fn then_delivery_outcome_surfaced(world: &mut TuiWorld) {
    with_harness(world, |h| {
        // The surfaced text must SPECIFICALLY reference the failed delivery
        // and the agent — an incidental unrelated notification cannot pass.
        let status = h.last_status_line().unwrap_or_default();
        let last_note = h.last_notification().unwrap_or_default();
        let surfaced = |s: &str| s.contains("not delivered") && s.contains("w1");
        assert!(
            surfaced(&status) || surfaced(&last_note),
            "a message to a dead/unattached sub-agent must surface a delivery \
             failure naming the agent; last status={status:?}, last \
             notification={last_note:?}"
        );
    });
}
