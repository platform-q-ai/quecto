//! App-level tests for the sourced fan-in routing seam (#1462).
//!
//! The event loop drains ONE fan-in channel keyed by `Source` instead of a
//! per-connection `client.recv()` select arm. These tests pin that routing a
//! master event / sub-agent event / Closed sentinel through the sourced path
//! behaves exactly like the pre-seam direct paths (N=1 behaviour unchanged,
//! #1047 disconnect diagnosis included).

use super::tui_harness::{self, TuiHarness, mask_clocks};
use crate::components::component::Component;
use crate::protocol::client::Event;

/// #1462 AC: a master event delivered via `SourcedEvent::Tab(MASTER)` renders in
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
        "a SourcedEvent::Tab(MASTER) event must reach the master session's chat (#1462)"
    );
    // Masked to `#:##` clocks: the Master row embeds a wall-clock uptime
    // timer, and a second boundary can pass between the two captures on a
    // loaded CI runner (#1462 flake fix).
    assert_eq!(
        mask_clocks(&got),
        mask_clocks(&expected),
        "N=1 frames must be byte-identical (up to wall-clock timers) between the direct path and the fan-in path (#1462)"
    );
}

/// #1462 review (falsifiability): the SAME parity, but end-to-end through
/// the production plumbing — event bytes written on the agent side of the
/// REAL master socket flow through the client reader, the connection feed
/// task, and the shared fan-in the event loop drains. A regression that
/// keeps a direct `client.recv()` path (or never spawns/drains the feed)
/// fails here, not just in the routing-level tests.
#[tokio::test]
async fn wire_master_event_flows_through_feed_task_and_fan_in() {
    let mut direct = TuiHarness::new().await;
    direct.event(Event::Token {
        token: "seam-wire-token".into(),
    });
    let expected = direct.full_frame();

    let mut wired = TuiHarness::new().await;
    wired
        .wire_master_event_line(r#"{"type":"token","token":"seam-wire-token"}"#)
        .await;
    let got = wired.full_frame();
    assert!(
        got.contains("seam-wire-token"),
        "an event written on the real socket must reach the master session via the feed task (#1462)"
    );
    assert_eq!(
        mask_clocks(&got),
        mask_clocks(&expected),
        "N=1 frames must be byte-identical (up to wall-clock timers) between direct handling and the full wire path (#1462)"
    );
}

/// #1462 review (falsifiability): closing the agent side of the REAL socket
/// (EOF) must surface as the feed task's Closed sentinel on the shared
/// fan-in and run the production disconnect handling.
#[tokio::test]
async fn wire_close_runs_disconnect_handling_via_closed_sentinel() {
    let mut h = TuiHarness::new().await;
    assert!(h.agent_connected(), "precondition: connected");

    h.wire_close_master_connection().await;

    assert!(
        !h.app_mut().active_conn().agent_connected,
        "a real EOF must mark the agent as not connected via the Closed sentinel (#1462)"
    );
    let messages = h.notification_messages().join("\n");
    assert!(
        messages.contains("Agent disconnected"),
        "a real EOF must raise the disconnect notification, got: {messages}"
    );
}

/// #1462 AC: a sub-agent event delivered via `SourcedEvent::Subagent(MASTER, id)`
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
        "a SourcedEvent::Subagent event must route into that sub-agent's session (#1462)"
    );
    assert_eq!(
        mask_clocks(&got),
        mask_clocks(&expected),
        "sub-agent frames must be identical (up to wall-clock timers) between direct routing and the fan-in path (#1462)"
    );
}

/// #1462 scope 3: the `SourcedEvent::Closed(MASTER)` sentinel runs the production
/// disconnect handling — agent marked disconnected, panel pinned (#1047),
/// disconnect notification shown.
#[tokio::test]
async fn closed_sentinel_marks_agent_disconnected() {
    let mut h = TuiHarness::new().await;
    assert!(h.agent_connected(), "precondition: connected");

    h.deliver_closed_sentinel().await;

    assert!(
        !h.app_mut().active_conn().agent_connected,
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

/// The clock mask must normalise exactly the wall-clock timer tokens
/// (`m:ss`, `h:mm:ss`) and nothing else — a mask that swallowed real
/// content would hollow out every parity assertion above.
#[test]
fn mask_clocks_masks_only_clock_tokens() {
    assert_eq!(
        mask_clocks("Master · idle 0:02 | ran 1:23:45 | v0.76 x1:2 seam-parity-token"),
        "Master · idle #:## | ran #:## | v0.76 x1:2 seam-parity-token"
    );
    assert_eq!(
        mask_clocks("idle 0:00"),
        mask_clocks("idle 12:59"),
        "two frames differing only in a timer must mask to the same bytes"
    );
}

// ── SourcedRender mapping (#1462 review: falsifiability) ──────────────
// The event loop's paint decision is the RETURN VALUE of `route_sourced`;
// the harness drivers capture unconditionally, so without these direct
// assertions the mapping could regress (e.g. every arm returning `Skip`
// freezes streamed paints; token arms returning `Immediate` reintroduces
// per-token flicker) with the whole suite still green.

use super::app_event_loop::SourcedRender;
use crate::shell::connection::{SourcedEvent, TabId};

#[tokio::test]
async fn route_sourced_ignores_master_event_for_foreign_tab() {
    let mut h = TuiHarness::new().await;
    h.app_mut().test_set_master_tab(1);

    let got = h.app_mut().route_sourced(SourcedEvent::Tab(
        TabId::MASTER,
        Event::Token {
            token: "foreign-master-token".into(),
        },
    ));
    h.capture();

    assert_eq!(
        got,
        SourcedRender::Stream { is_token: true },
        "foreign master events are still classified for the stream paint path"
    );
    assert!(
        !h.full_frame().contains("foreign-master-token"),
        "a SourcedEvent::Tab tagged for a different tab must not mutate the active connection (#1472 sweep)"
    );
}

#[tokio::test]
async fn route_sourced_master_token_coalesces_as_stream_token() {
    let mut h = TuiHarness::new().await;
    let got = h.app_mut().route_sourced(SourcedEvent::Tab(
        TabId::MASTER,
        Event::Token { token: "t".into() },
    ));
    assert_eq!(
        got,
        SourcedRender::Stream { is_token: true },
        "a master token must take the coalesced stream paint path (#1462)"
    );
}

#[tokio::test]
async fn route_sourced_master_non_token_paints_stream_immediately() {
    let mut h = TuiHarness::new().await;
    let got = h
        .app_mut()
        .route_sourced(SourcedEvent::Tab(TabId::MASTER, Event::TurnStart));
    assert_eq!(
        got,
        SourcedRender::Stream { is_token: false },
        "a non-token master event must stream-paint without coalescing (#1462)"
    );
}

#[tokio::test]
async fn route_sourced_master_event_with_surfaced_drops_paints_immediately() {
    let mut h = TuiHarness::new().await;
    h.app_mut()
        .active_conn()
        .transport
        .record_dropped_oversized_for_tests(1);
    let got = h.app_mut().route_sourced(SourcedEvent::Tab(
        TabId::MASTER,
        Event::Token { token: "t".into() },
    ));
    assert_eq!(
        got,
        SourcedRender::Immediate,
        "surfacing an oversized-line drop must force an immediate paint (#1047)"
    );
}

#[tokio::test]
async fn route_sourced_ignores_subagent_event_for_foreign_tab() {
    let mut h = TuiHarness::new().await;
    h.app_mut().test_set_master_tab(1);
    h.event(tui_harness::subagents_changed(vec![tui_harness::subagent(
        "foreign-a1",
        "running",
        None,
    )]));
    h.select(Some("foreign-a1"));

    let got = h.app_mut().route_sourced(SourcedEvent::Subagent(
        TabId::MASTER,
        "foreign-a1".into(),
        Event::Token {
            token: "foreign-tab-token".into(),
        },
    ));
    h.capture();

    assert_eq!(
        got,
        SourcedRender::Stream { is_token: true },
        "foreign sub-agent events are still classified for the stream paint path"
    );
    assert!(
        !h.full_frame().contains("foreign-tab-token"),
        "a SourcedEvent::Subagent tagged for a different tab must not mutate the active connection (#1472)"
    );
}

#[tokio::test]
async fn route_sourced_subagent_token_coalesces_as_stream_token() {
    let mut h = TuiHarness::new().await;
    let got = h.app_mut().route_sourced(SourcedEvent::Subagent(
        TabId::MASTER,
        "a1".into(),
        Event::Token { token: "t".into() },
    ));
    assert_eq!(
        got,
        SourcedRender::Stream { is_token: true },
        "a sub-agent token must take the coalesced stream paint path (#1462)"
    );
}

#[tokio::test]
async fn route_sourced_closed_sentinel_paints_immediately() {
    let mut h = TuiHarness::new().await;
    let got = h
        .app_mut()
        .route_sourced(SourcedEvent::Closed(TabId::MASTER));
    assert_eq!(
        got,
        SourcedRender::Immediate,
        "the disconnect sentinel must paint immediately, not coalesce (#1462)"
    );
}

// ── run() drains the shared fan-in (#1462 review: falsifiability) ─────
// Every other test pumps the fan-in through harness helpers, so the select
// arm in `run()` itself had no coverage: deleting it — or re-gating it the
// way the removed `client.recv()` arm was gated on `agent_connected` —
// would pass the whole suite while the production TUI stopped processing
// events. This drives the REAL event loop and lets IT deliver the item.

#[tokio::test]
async fn run_select_loop_drains_tab_fan_in_even_when_disconnected() {
    let mut h = TuiHarness::new().await;
    // Pre-flip the flag the deleted `client.recv()` arm used to be gated on,
    // then drive the item through the REAL master wire so it travels client
    // reader → feed task → `tab_event_tx` → the dedicated tab select arm —
    // the actual replacement for that gated arm (#1470 review). Injecting a
    // hand-built `SourcedEvent` on the sub-agent channel would instead drain
    // through the always-unconditional sub-agent arm and never exercise the
    // tab arm, so re-gating the tab arm on `agent_connected` (which would
    // starve the master feed and the `Closed` sentinel after a disconnect)
    // would ship uncaught. Driving the tab arm here makes that regression fail.
    h.app_mut().active_conn_mut().agent_connected = false;
    h.send_agent_event_line("{\"type\":\"token\",\"token\":\"tab-arm-disconnected-token\"}")
        .await;

    // run() never exits on its own here; give the loop a moment to select
    // the queued item, then drop the future.
    let _ = tokio::time::timeout(std::time::Duration::from_millis(500), h.app_mut().run()).await;

    h.capture();
    assert!(
        h.full_frame().contains("tab-arm-disconnected-token"),
        "run()'s select loop must drain the dedicated tab fan-in unconditionally, \
         even while disconnected (#1470)"
    );
}

/// The dedicated master/tab fan-in arm (#1470 review) must equally be
/// drained by run() itself: an event written on the REAL master socket
/// travels client reader → feed task → tab channel → run()'s select arm.
#[tokio::test]
async fn run_select_loop_drains_tab_fan_in_from_the_wire() {
    let mut h = TuiHarness::new().await;
    h.send_agent_event_line("{\"type\":\"token\",\"token\":\"tab-arm-run-token\"}")
        .await;

    let _ = tokio::time::timeout(std::time::Duration::from_millis(500), h.app_mut().run()).await;

    h.capture();
    assert!(
        h.full_frame().contains("tab-arm-run-token"),
        "run()'s select loop must drain the dedicated tab fan-in (#1470)"
    );
}
