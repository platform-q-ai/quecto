//! Steps for `tui_esc_abort_recovery.feature`.
//!
//! Drives the REAL abort + submit + event-handling paths through the headless
//! render harness to prove an Esc abort is non-destructive: a subsequent prompt
//! is still dispatched over the socket and its streamed response is rendered,
//! and stale/compounded aborts are absorbed by the abort-aware state machine.
//!
//! (The `the agent is streaming a response` Given is shared and defined in
//! `tui_ctrl_d_exit_steps`.)

use crate::{TuiParityHarness, TuiWorld};
use cucumber::{given, then, when};
use quecto_tui::infrastructure::client::Event;
use quecto_tui::interface::app::tui_harness::TuiHarness;
use quecto_tui::shell::keys::Key;

const RESPONSE_TEXT: &str = "Here is the continued response";

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

fn drain(world: &mut TuiWorld) -> Vec<String> {
    let rt = world.tui_parity_rt.as_ref().expect("runtime");
    let h = &mut world.tui_parity.as_mut().expect("harness").0;
    rt.block_on(h.drain_commands())
}

/// Submit `prompt` through the real submit path, drain the dispatched socket
/// command, then feed the agent's streamed response back through the real event
/// handler so the response is rendered.
fn submit_and_respond(world: &mut TuiWorld, prompt: &str) {
    // Discard any commands queued by prior steps (e.g. the abort) so we observe
    // only this prompt's dispatch, not leftover abort frames.
    let _ = drain(world);
    with_harness(world, |h| {
        h.submit(prompt);
    });
    // `send_command` dispatches via a fire-and-forget spawn, so under sharded
    // load the prompt can lag (and a late abort may arrive first). Accumulate
    // drains until the prompt command appears, bounded so a genuine miss fails.
    let mut cmds = Vec::new();
    for _ in 0..10 {
        cmds.extend(drain(world));
        if cmds.iter().any(|c| c.contains("\"type\":\"prompt\"")) {
            break;
        }
    }
    world.tui_last_commands = cmds;
    with_harness(world, |h| {
        h.event(Event::AgentStart);
        h.event(Event::Token {
            token: RESPONSE_TEXT.to_string(),
        });
        h.event(Event::TurnEnd {
            message: serde_json::json!({ "role": "assistant", "content": RESPONSE_TEXT }),
        });
        h.event(Event::AgentEnd {
            messages: vec![],
            message_refs: vec![],
        });
    });
}

// ── Given ──────────────────────────────────────────────────────────────────

#[given("a stale abort fired before the prompt started")]
fn stale_abort(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.event(Event::AgentStart);
        h.abort();
    });
    let pending = with_harness(world, |h| h.pending_aborts());
    assert_eq!(
        pending, 1,
        "precondition: one stale abort should be pending"
    );
}

#[given("the user aborts 3 times in rapid succession")]
fn aborts_three_times(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.event(Event::AgentStart);
        h.abort();
        h.abort();
        h.abort();
    });
    // Aborts must NOT compound: a single run yields at most one pending abort.
    let pending = with_harness(world, |h| h.pending_aborts());
    assert_eq!(
        pending, 1,
        "3 rapid aborts on one run must not compound the pending count, got {pending}"
    );
}

// ── When ───────────────────────────────────────────────────────────────────

#[when("the user presses Escape to abort")]
fn press_escape(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.press(Key::Escape);
    });
    let (running, pending) = with_harness(world, |h| (h.agent_running(), h.pending_aborts()));
    assert!(
        !running && pending >= 1,
        "Escape should have aborted the run"
    );
}

#[when("then submits a new prompt")]
fn submits_new_prompt(world: &mut TuiWorld) {
    submit_and_respond(world, "continue the task");
}

#[when("the TUI sends a prompt")]
fn tui_sends_prompt(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.submit("run the next step");
    });
    let cmds = drain(world);
    world.tui_last_commands = cmds;
}

#[when("the user sends a new prompt")]
fn user_sends_new_prompt(world: &mut TuiWorld) {
    submit_and_respond(world, "start a fresh task");
}

// ── Then ───────────────────────────────────────────────────────────────────

#[then("the agent should process the new prompt")]
fn agent_processes_prompt(world: &mut TuiWorld) {
    assert!(
        world
            .tui_last_commands
            .iter()
            .any(|c| c.contains("\"type\":\"prompt\"")),
        "a prompt command should have been dispatched over the socket, got: {:?}",
        world.tui_last_commands
    );
}

#[then("the user should see a response")]
fn user_sees_response(world: &mut TuiWorld) {
    let frame = with_harness(world, |h| h.full_frame());
    assert!(
        frame.contains(RESPONSE_TEXT),
        "the streamed response should be rendered, frame:\n{frame}"
    );
}

#[then("the agent should send an agent_end event")]
fn agent_sends_agent_end(world: &mut TuiWorld) {
    // Deliver the stale AgentEnd from the pre-cancelled run. The abort-aware
    // state machine must consume the pending abort rather than corrupt state.
    with_harness(world, |h| {
        h.event(Event::AgentEnd {
            messages: vec![],
            message_refs: vec![],
        });
    });
    let pending = with_harness(world, |h| h.pending_aborts());
    assert_eq!(
        pending, 0,
        "the stale AgentEnd should have consumed the pending abort, got {pending}"
    );
}

#[then("the TUI should not hang waiting for a response")]
fn tui_does_not_hang(world: &mut TuiWorld) {
    // Not hung: the machine is idle (not stuck "running"), and a fresh run
    // streams a response that renders normally.
    assert!(
        !with_harness(world, |h| h.agent_running()),
        "the TUI must not be stuck in a running state"
    );
    with_harness(world, |h| {
        h.event(Event::AgentStart);
        h.event(Event::Token {
            token: RESPONSE_TEXT.to_string(),
        });
        h.event(Event::AgentEnd {
            messages: vec![],
            message_refs: vec![],
        });
    });
    let frame = with_harness(world, |h| h.full_frame());
    assert!(
        frame.contains(RESPONSE_TEXT),
        "a subsequent run should render its response (no hang), frame:\n{frame}"
    );
}

#[then("the prompt should be processed normally")]
fn prompt_processed_normally(world: &mut TuiWorld) {
    assert!(
        world
            .tui_last_commands
            .iter()
            .any(|c| c.contains("\"type\":\"prompt\"")),
        "the new prompt should have been dispatched, got: {:?}",
        world.tui_last_commands
    );
    let (running, pending, frame) = with_harness(world, |h| {
        (h.agent_running(), h.pending_aborts(), h.full_frame())
    });
    assert!(!running, "the run should have ended cleanly");
    assert_eq!(pending, 0, "no stale abort should linger after a clean run");
    assert!(
        frame.contains(RESPONSE_TEXT),
        "the response should be rendered normally, frame:\n{frame}"
    );
}
