//! Unit tests for agent-disconnect handling (#1047): a dying agent (e.g. a
//! panic-abort near a full context window) must not silently strip the UI of
//! context — the left panel stays visible and the disconnect notification
//! carries the child's exit diagnosis.

use super::tui_harness::TuiHarness;
use crate::components::component::Component;

/// #1047 AC2: once connected, the persistent left panel must survive an agent
/// disconnect (shown in a "disconnected" state) rather than vanishing, so the
/// user keeps the session/sub-agent context needed to diagnose the failure.
#[tokio::test]
async fn left_panel_stays_visible_after_agent_disconnect() {
    let mut h = TuiHarness::new().await;
    let app = h.app_mut();
    assert!(
        app.subagent_panel_visible(),
        "precondition: panel is visible while connected"
    );

    app.handle_agent_disconnected(None);

    assert!(
        !app.agent_connected,
        "disconnect must still mark the agent as not connected"
    );
    assert!(
        app.subagent_panel_visible(),
        "left panel must remain visible after the agent disconnects (#1047)"
    );
}

/// #1047 AC1: when the TUI owns the agent child and it died, the disconnect
/// notification must include the exit diagnosis (status/signal), not just a
/// bare "Agent disconnected".
#[tokio::test]
async fn disconnect_notification_includes_agent_exit_detail() {
    let mut h = TuiHarness::new().await;
    let app = h.app_mut();

    app.handle_agent_disconnected(Some(
        "agent process aborted: signal 6 (SIGABRT)".to_string(),
    ));

    let rendered = app.notifications.render(200).join("\n");
    assert!(
        rendered.contains("SIGABRT"),
        "disconnect notification must surface the child's exit detail (#1047): {rendered}"
    );
}

/// #1047 AC1 wiring: the PRODUCTION disconnect path (`handle_agent_stream_closed`)
/// must read the exit diagnosis from the child watcher's slot — for a real
/// spawned child killed by a real signal — not be hand-fed a string.
#[tokio::test]
async fn stream_closed_path_reports_real_child_exit_detail() {
    let mut h = TuiHarness::new().await;
    let app = h.app_mut();

    let child = tokio::process::Command::new("sh")
        .args(["-c", "kill -ABRT $$"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn aborting child");
    app.set_child_exit_watch(crate::shell::child_watch::watch_child(
        child,
        crate::shell::child_watch::StderrTail::default(),
    ));

    app.handle_agent_stream_closed().await;

    let rendered = app.notifications.render(200).join("\n");
    assert!(
        rendered.contains("signal 6 (SIGABRT)"),
        "the stream-closed path must diagnose the real child's abort (#1047): {rendered}"
    );
}

/// #1047 (top-priority fix): the agent's post-startup stderr is drained into
/// the watcher's ring buffer, and the disconnect diagnostics include the tail
/// — under `panic = "abort"` the panic message lands on stderr right before
/// the process dies, and without this every recurrence is undiagnosable.
#[tokio::test]
async fn disconnect_diagnostics_include_drained_stderr_tail() {
    use crate::shell::child_watch::{StderrTail, watch_child};

    let mut h = TuiHarness::new().await;

    let mut child = tokio::process::Command::new("sh")
        .args([
            "-c",
            "echo \"thread 'main' panicked at boom-panic\" >&2; kill -ABRT $$",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn aborting child");
    let stderr = child.stderr.take().expect("child stderr");
    let tail = StderrTail::default();
    crate::shell::cli::spawn_stderr_drain(tokio::io::BufReader::new(stderr), tail.clone());
    let watch = watch_child(child, tail.clone());
    // Deliberately NO wait for the drain here: the disconnect path itself
    // must synchronize on drain completion, or the panic message is
    // nondeterministically missing from the diagnostics (#1051 final review).

    let app = h.app_mut();
    app.set_child_exit_watch(watch);
    app.handle_agent_stream_closed().await;

    let rendered = app.notifications.render(200).join("\n");
    assert!(
        rendered.contains("boom-panic"),
        "disconnect notification must carry the last stderr line (#1047): {rendered}"
    );
    assert!(
        rendered.contains("SIGABRT"),
        "exit diagnosis must still be present alongside the stderr tail: {rendered}"
    );
}

/// #1047 AC4: a dropped oversized event must be SURFACED in the UI — a
/// counted-but-silent drop still leaves the session looking frozen.
#[tokio::test]
async fn oversized_event_drop_is_surfaced_as_notification() {
    let mut h = TuiHarness::new().await;
    let app = h.app_mut();

    assert!(
        !app.surface_dropped_oversized_events(),
        "no drops recorded — nothing to surface"
    );

    app.client.record_dropped_oversized_for_tests(1);
    assert!(
        app.surface_dropped_oversized_events(),
        "a recorded drop must raise a notification"
    );
    let rendered = app.notifications.render(200).join("\n");
    assert!(
        rendered.contains("oversized agent event"),
        "the drop notification must name the oversized-event loss (#1047): {rendered}"
    );

    assert!(
        !app.surface_dropped_oversized_events(),
        "the same drop must not be surfaced twice"
    );
}
