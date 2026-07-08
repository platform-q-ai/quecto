//! Step definitions for `tui_agent_disconnect_diagnostics.feature` (#1047).
//!
//! These drive the REAL TUI disconnect path through the headless render
//! harness: a live `App` whose agent event stream closes, and — for the
//! exit-diagnosis scenario — a REAL spawned child process killed by a real
//! signal, reaped by the production child watcher.

use super::*;
use quecto_tui::infrastructure::child_watch::{self, ExitDetailSlot};
use quecto_tui::interface::app::tui_harness::TuiHarness;

/// A real spawned "agent" child under the production exit watcher.
#[derive(Debug)]
pub struct DisconnectChildWatch {
    /// PID to signal in the When step.
    pub pid: u32,
    /// Slot the watcher fills with the exit diagnosis once the child is reaped.
    pub slot: ExitDetailSlot,
}

fn init_disconnect_harness(world: &mut TuiWorld) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let h = rt.block_on(TuiHarness::new());
    world.tui_parity_rt = Some(rt);
    world.tui_parity = Some(TuiParityHarness(h));
}

fn harness(world: &mut TuiWorld) -> &mut TuiHarness {
    &mut world.tui_parity.as_mut().expect("TUI harness").0
}

#[given("the TUI is connected to an agent with the left panel visible")]
fn tui_connected_with_left_panel(world: &mut TuiWorld) {
    init_disconnect_harness(world);
    let h = harness(world);
    assert!(h.agent_connected(), "precondition: agent is connected");
    assert!(
        h.subagent_panel_visible(),
        "precondition: the left panel is visible while connected"
    );
}

#[when("the agent connection closes unexpectedly")]
fn agent_connection_closes_unexpectedly(world: &mut TuiWorld) {
    harness(world).agent_stream_closed();
}

#[then("the left panel should remain visible")]
fn left_panel_remains_visible(world: &mut TuiWorld) {
    let h = harness(world);
    assert!(
        !h.agent_connected(),
        "the disconnect must still mark the agent as not connected"
    );
    assert!(
        h.subagent_panel_visible(),
        "the left panel must remain visible after the agent disconnects (#1047)"
    );
}

#[then("the TUI should show a disconnect notification")]
fn tui_shows_disconnect_notification(world: &mut TuiWorld) {
    let text = harness(world).notification_text();
    assert!(
        text.contains("Agent disconnected"),
        "expected a disconnect notification, got: {text}"
    );
}

#[given("the TUI spawned its own agent child process")]
fn tui_spawned_agent_child(world: &mut TuiWorld) {
    init_disconnect_harness(world);
    let rt = world.tui_parity_rt.as_ref().expect("runtime");
    let watch = rt.block_on(async {
        // A stand-in agent child that would run forever unless killed.
        let child = tokio::process::Command::new("sleep")
            .arg("600")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn agent stand-in child");
        let pid = child.id().expect("child pid");
        let slot = child_watch::watch_child(child);
        DisconnectChildWatch { pid, slot }
    });
    world.tui_disconnect_child = Some(watch);
}

#[when("the agent child process aborts with a signal")]
fn agent_child_aborts_with_signal(world: &mut TuiWorld) {
    let watch = world
        .tui_disconnect_child
        .take()
        .expect("spawned agent child");
    // Abort the REAL child with a real SIGABRT; its death closes the agent
    // connection, which the TUI observes as the stream closing.
    // SAFETY: pid comes from a child we just spawned; kill with a valid
    // signal returns an error code rather than faulting.
    let rc = unsafe { libc::kill(watch.pid as i32, libc::SIGABRT) };
    assert_eq!(rc, 0, "SIGABRT must be delivered to the spawned child");

    let rt = world.tui_parity_rt.as_ref().expect("runtime");
    let handle = rt.handle().clone();
    let h = &mut world.tui_parity.as_mut().expect("TUI harness").0;
    handle.block_on(h.agent_stream_closed_with_child_watch(watch.slot));
}

#[then("the disconnect notification should include the child's exit detail")]
fn disconnect_notification_includes_exit_detail(world: &mut TuiWorld) {
    let text = harness(world).notification_text();
    assert!(
        text.contains("signal 6 (SIGABRT)"),
        "the disconnect notification must diagnose the child's abort (#1047), got: {text}"
    );
}
