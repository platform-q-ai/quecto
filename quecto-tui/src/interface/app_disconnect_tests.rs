//! Unit tests for agent-disconnect handling (#1047): a dying agent (e.g. a
//! panic-abort near a full context window) must not silently strip the UI of
//! context — the left panel stays visible and the disconnect notification
//! carries the child's exit diagnosis.

use super::tui_harness::TuiHarness;
use crate::interface::component::Component;

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
    app.set_child_exit_watch(crate::infrastructure::child_watch::watch_child(child));

    app.handle_agent_stream_closed().await;

    let rendered = app.notifications.render(200).join("\n");
    assert!(
        rendered.contains("signal 6 (SIGABRT)"),
        "the stream-closed path must diagnose the real child's abort (#1047): {rendered}"
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
