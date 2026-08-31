//! Steps for `tui_ctrl_d_exit.feature` (#478).
//!
//! Drives the REAL `App::handle_key` Ctrl+D path through the headless render
//! harness. Ctrl+D exits unconditionally — regardless of overlays or
//! autocomplete — via the ordinary-exit path.
//!
//! Also owns the shared `the agent is streaming a response` Given (reused by
//! `tui_esc_abort_recovery.feature`), which starts a real agent run.

use crate::{TuiParityHarness, TuiWorld};
use cucumber::{given, then, when};
use quecto_tui::protocol::client::Event;
use quecto_tui::shell::app::tui_harness::TuiHarness;
use quecto_tui::shell::keys::Key;

fn with_harness<R>(world: &mut TuiWorld, f: impl FnOnce(&mut TuiHarness) -> R) -> R {
    if world.tui_parity_rt.is_none() {
        world.tui_parity_rt = Some(tokio::runtime::Runtime::new().expect("tokio runtime"));
    }
    if world.tui_parity.is_none() {
        let rt = world.tui_parity_rt.as_ref().expect("runtime");
        let h = rt.block_on(async { TuiHarness::new().await });
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

// ── Given ──────────────────────────────────────────────────────────────────

#[given("the TUI is running with no overlays")]
fn tui_no_overlays(world: &mut TuiWorld) {
    // Fresh harness: no overlay, not exiting.
    let (exit, overlay) = with_harness(world, |h| (h.should_exit(), h.model_selector_open()));
    assert!(!exit && !overlay, "precondition: idle TUI, no overlays");
}

#[given("a confirm overlay is active")]
fn confirm_overlay_active(world: &mut TuiWorld) {
    with_harness(world, |h| h.open_overlay());
    assert!(
        with_harness(world, |h| h.model_selector_open()),
        "precondition: an overlay should be active"
    );
}

/// Shared with `tui_esc_abort_recovery.feature`: start a real agent run so the
/// master session is streaming (running + streaming footer + spinner).
#[given("the agent is streaming a response")]
fn agent_streaming(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.event(Event::AgentStart);
    });
    assert!(
        with_harness(world, |h| h.agent_running()),
        "precondition: agent should be streaming/running"
    );
}

#[given("the autocomplete dropdown is showing")]
fn autocomplete_showing(world: &mut TuiWorld) {
    // Typing '/' through the real key path opens the slash-command autocomplete.
    with_harness(world, |h| {
        h.press(Key::Char('/'));
    });
    assert!(
        with_harness(world, |h| h.autocomplete_active()),
        "precondition: the slash-command autocomplete should be active"
    );
}

// ── When ───────────────────────────────────────────────────────────────────

#[when("the user presses Ctrl+D")]
fn press_ctrl_d(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.press(Key::Ctrl('d'));
    });
}

// ── Then ───────────────────────────────────────────────────────────────────

#[then("the app should set should_exit to true")]
fn should_exit_true(world: &mut TuiWorld) {
    assert!(
        with_harness(world, |h| h.should_exit()),
        "Ctrl+D must set the app's exit flag"
    );
}

#[then("the main loop should break")]
fn main_loop_breaks(world: &mut TuiWorld) {
    // The event loop's top-of-loop guard is `if self.should_exit { break; }`,
    // so a set exit flag is exactly the condition under which the loop breaks.
    assert!(
        with_harness(world, |h| h.should_exit()),
        "with should_exit set, the main loop's guard breaks the loop"
    );
}

#[then(regex = r"^(?:then )?the app should exit$")]
fn the_app_should_exit(world: &mut TuiWorld) {
    assert!(
        with_harness(world, |h| h.should_exit()),
        "the app should have been asked to exit"
    );
}

#[then("the overlay should not consume the key")]
fn overlay_did_not_consume(world: &mut TuiWorld) {
    // Ctrl+D is handled at the very top of `handle_key`, before overlay routing,
    // so the overlay never processes it: it stays open AND the exit flag is set.
    let (exit, overlay_open) = with_harness(world, |h| (h.should_exit(), h.model_selector_open()));
    assert!(exit, "Ctrl+D must exit even with an overlay active");
    assert!(
        overlay_open,
        "the overlay must not have consumed the key (it should remain open)"
    );
}

#[then("the active agent should continue without a Ctrl-D abort")]
fn active_agent_continues_without_ctrl_d_abort(world: &mut TuiWorld) {
    let (running, pending) = with_harness(world, |h| (h.agent_running(), h.pending_aborts()));
    assert!(
        running,
        "Ctrl-D ordinary exit should not abort the running agent"
    );
    assert_eq!(
        pending, 0,
        "Ctrl-D ordinary exit should not register a pending abort"
    );
    let frame = with_harness(world, |h| h.full_frame());
    assert!(
        !frame.contains("Operation aborted"),
        "Ctrl-D ordinary exit should not surface an abort status, frame:\n{frame}"
    );
}
