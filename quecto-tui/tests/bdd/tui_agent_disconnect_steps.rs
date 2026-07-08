//! Step definitions for `tui_agent_disconnect_diagnostics.feature` (#1047).
//!
//! These drive the REAL TUI disconnect path through the headless render
//! harness: a live `App` whose agent event stream closes, and — for the
//! exit-diagnosis scenario — a REAL spawned child process killed by a real
//! signal, reaped by the production child watcher.

use super::*;
use quecto_tui::infrastructure::child_watch::{self, ChildWatch};
use quecto_tui::interface::app::tui_harness::TuiHarness;

/// A real spawned "agent" child under the production exit watcher.
#[derive(Debug)]
pub struct DisconnectChildWatch {
    /// PID to signal in the When step.
    pub pid: u32,
    /// Watch handle the watcher publishes the exit diagnosis on once the
    /// child is reaped.
    pub watch: ChildWatch,
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
        let watch = child_watch::watch_child(child, child_watch::StderrTail::default());
        DisconnectChildWatch { pid, watch }
    });
    world.tui_disconnect_child = Some(watch);
}

/// #1047: an agent stand-in whose stderr carries a panic-style message
/// mid-session, drained by the PRODUCTION post-startup stderr drain.
#[given("the TUI spawned its own agent child process that writes a panic message to stderr")]
fn tui_spawned_agent_child_with_panicky_stderr(world: &mut TuiWorld) {
    init_disconnect_harness(world);
    let rt = world.tui_parity_rt.as_ref().expect("runtime");
    let watch = rt.block_on(async {
        let mut child = tokio::process::Command::new("sh")
            .args(["-c", "echo 'panicked: boom-panic' >&2; sleep 600"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn agent stand-in child");
        let pid = child.id().expect("child pid");
        let stderr = child.stderr.take().expect("child stderr");
        let tail = child_watch::StderrTail::default();
        quecto_tui::interface::cli::spawn_stderr_drain(
            tokio::io::BufReader::new(stderr),
            tail.clone(),
        );
        let watch = child_watch::watch_child(child, tail);
        // Wait until the production drain has captured the panic line, so the
        // later abort exercises the diagnostics path deterministically.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        while watch.stderr_tail_lines().is_empty() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "the stderr drain must capture the panic line"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        DisconnectChildWatch { pid, watch }
    });
    world.tui_disconnect_child = Some(watch);
}

#[then("the disconnect diagnostics should include the panic message from stderr")]
fn disconnect_diagnostics_include_panic_message(world: &mut TuiWorld) {
    let text = harness(world).notification_text();
    assert!(
        text.contains("boom-panic"),
        "the disconnect notification must carry the agent's last stderr line (#1047), got: {text}"
    );
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
    handle.block_on(h.agent_stream_closed_with_child_watch(watch.watch));
}

#[then("the disconnect notification should include the child's exit detail")]
fn disconnect_notification_includes_exit_detail(world: &mut TuiWorld) {
    let text = harness(world).notification_text();
    assert!(
        text.contains("signal 6 (SIGABRT)"),
        "the disconnect notification must diagnose the child's abort (#1047), got: {text}"
    );
}
