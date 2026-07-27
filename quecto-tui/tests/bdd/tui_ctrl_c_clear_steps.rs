//! Steps for `tui_ctrl_c_clear.feature` (#536).
//!
//! Drives the REAL `App::handle_key` Ctrl+C path through the headless render
//! harness: Ctrl+C clears the editor when it has content, and only aborts a
//! running agent when the editor is already empty.

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

#[given("the agent is running")]
fn agent_running(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.event(Event::AgentStart);
    });
    assert!(
        with_harness(world, |h| h.agent_running()),
        "precondition: agent should be running"
    );
}

#[given("the agent is idle")]
fn agent_idle(world: &mut TuiWorld) {
    // A fresh harness starts idle; assert the real state to make it explicit.
    assert!(
        !with_harness(world, |h| h.agent_running()),
        "precondition: agent should be idle"
    );
}

#[given(regex = r#"^the editor contains "([^"]*)"$"#)]
fn editor_contains(world: &mut TuiWorld, text: String) {
    with_harness(world, |h| h.set_editor_text(&text));
    assert_eq!(
        with_harness(world, |h| h.editor_text()),
        text,
        "precondition: editor should hold the given text"
    );
}

#[given("the editor is empty")]
fn editor_is_empty(world: &mut TuiWorld) {
    with_harness(world, |h| h.set_editor_text(""));
    assert!(
        with_harness(world, |h| h.editor_text()).is_empty(),
        "precondition: editor should be empty"
    );
}

// ── When ───────────────────────────────────────────────────────────────────

#[when("the user presses Ctrl+C")]
fn press_ctrl_c(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.press(Key::Ctrl('c'));
    });
}

// ── Then ───────────────────────────────────────────────────────────────────

#[then("the editor should be empty")]
fn editor_should_be_empty(world: &mut TuiWorld) {
    let text = with_harness(world, |h| h.editor_text());
    assert!(text.is_empty(), "editor should be empty, got {text:?}");
}

#[then("the agent should still be running")]
fn agent_still_running(world: &mut TuiWorld) {
    assert!(
        with_harness(world, |h| h.agent_running()),
        "agent should still be running after Ctrl+C cleared the editor"
    );
}

#[then("the agent should be aborted")]
fn agent_should_be_aborted(world: &mut TuiWorld) {
    let (running, pending) = with_harness(world, |h| (h.agent_running(), h.pending_aborts()));
    assert!(!running, "agent should no longer be running after abort");
    assert!(
        pending >= 1,
        "handle_abort should have registered a pending abort, got {pending}"
    );
}

#[then("the agent should still be idle")]
fn agent_still_idle(world: &mut TuiWorld) {
    let (running, pending) = with_harness(world, |h| (h.agent_running(), h.pending_aborts()));
    assert!(!running, "agent should still be idle");
    assert_eq!(
        pending, 0,
        "an idle Ctrl+C must not register a pending abort, got {pending}"
    );
}
