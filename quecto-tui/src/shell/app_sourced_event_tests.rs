//! App-level tests for the sourced fan-in routing seam (#1462).
//!
//! The event loop drains ONE fan-in channel keyed by `Source` instead of a
//! per-connection `client.recv()` select arm. These tests pin that routing a
//! master event / sub-agent event / Closed sentinel through the sourced path
//! behaves exactly like the pre-seam direct paths (N=1 behaviour unchanged,
//! #1047 disconnect diagnosis included).

use super::tui_harness::{self, TuiHarness};
use crate::components::component::Component;
use crate::protocol::client::Event;

/// #1462 AC: a master event delivered via `Source::Tab(MASTER)` renders in
/// the master session exactly as a directly handled event would.
#[tokio::test]
async fn sourced_master_event_renders_like_direct_handling() {
    let mut direct = TuiHarness::new().await;
    direct.event(Event::Token {
        token: "seam-parity-token".into(),
    });
    let expected = direct.full_frame();
    assert!(
        expected.contains("seam-parity-token"),
        "baseline: direct handling renders the token"
    );

    let mut sourced = TuiHarness::new().await;
    sourced
        .sourced_master_event(Event::Token {
            token: "seam-parity-token".into(),
        })
        .await;
    let got = sourced.full_frame();
    assert!(
        got.contains("seam-parity-token"),
        "a Source::Tab(MASTER) event must reach the master session's chat (#1462)"
    );
    assert_eq!(
        got, expected,
        "N=1 frames must be byte-identical between the direct path and the fan-in path (#1462)"
    );
}

/// #1462 AC: a sub-agent event delivered via `Source::Subagent(MASTER, id)`
/// routes into that sub-agent's session like `route_subagent_event` does.
#[tokio::test]
async fn sourced_subagent_event_routes_to_subagent_session() {
    let mut direct = TuiHarness::new().await;
    direct.event(tui_harness::subagents_changed(vec![tui_harness::subagent(
        "seam-a1", "running", None,
    )]));
    direct.select(Some("seam-a1"));
    direct.route(
        "seam-a1",
        Event::Token {
            token: "seam-subagent-token".into(),
        },
    );
    let expected = direct.full_frame();
    assert!(
        expected.contains("seam-subagent-token"),
        "baseline: direct routing renders the sub-agent token"
    );

    let mut sourced = TuiHarness::new().await;
    sourced.event(tui_harness::subagents_changed(vec![tui_harness::subagent(
        "seam-a1", "running", None,
    )]));
    sourced.select(Some("seam-a1"));
    sourced
        .sourced_subagent_event(
            "seam-a1",
            Event::Token {
                token: "seam-subagent-token".into(),
            },
        )
        .await;
    let got = sourced.full_frame();
    assert!(
        got.contains("seam-subagent-token"),
        "a Source::Subagent event must route into that sub-agent's session (#1462)"
    );
    assert_eq!(
        got, expected,
        "sub-agent frames must be identical between direct routing and the fan-in path (#1462)"
    );
}

/// #1462 scope 3: the `Source::Closed(MASTER)` sentinel runs the production
/// disconnect handling — agent marked disconnected, panel pinned (#1047),
/// disconnect notification shown.
#[tokio::test]
async fn closed_sentinel_marks_agent_disconnected() {
    let mut h = TuiHarness::new().await;
    assert!(h.agent_connected(), "precondition: connected");

    h.deliver_closed_sentinel().await;

    assert!(
        !h.app_mut().agent_connected,
        "the Closed sentinel must mark the agent as not connected (#1462)"
    );
    assert!(
        h.subagent_panel_visible(),
        "the left panel must remain visible after the sentinel (#1047 unchanged)"
    );
    let messages = h.notification_messages().join("\n");
    assert!(
        messages.contains("Agent disconnected"),
        "the Closed sentinel must raise the disconnect notification, got: {messages}"
    );
}

/// #1462 scope 3 + #1047: the sentinel path still reads the owned child's
/// exit diagnosis from the watcher — for a real child killed by a real
/// signal — even though the diagnosis await moved off the select loop.
#[tokio::test]
async fn closed_sentinel_reports_real_child_exit_detail() {
    let mut h = TuiHarness::new().await;

    let child = tokio::process::Command::new("sh")
        .args(["-c", "kill -ABRT $$"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn aborting child");
    let watch = crate::shell::child_watch::watch_child(
        child,
        crate::shell::child_watch::StderrTail::default(),
    );

    h.deliver_closed_sentinel_with_child_watch(watch).await;

    let rendered = h.app_mut().notifications.render(200).join("\n");
    assert!(
        rendered.contains("signal 6 (SIGABRT)"),
        "the Closed sentinel path must diagnose the real child's abort (#1047 via #1462): {rendered}"
    );
}
