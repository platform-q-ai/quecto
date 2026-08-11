//! Step definitions for `tui_master_connection_feed.feature` (#1462).
//!
//! Phase 1 of the multi-session TUI (epic #1467): the master connection moves
//! behind a feed task and its events arrive through the shared fan-in channel
//! keyed by `Source`. At N=1 nothing may change: these steps pin frame parity
//! between the direct path and the feed path, and that a real stream close —
//! delivered as the feed task's `Source::Closed` sentinel — keeps the #1047
//! disconnect diagnosis. The When steps drive the REAL socket: event bytes
//! (or EOF) travel client reader → feed task → shared fan-in → routing.

use super::*;
use quecto_tui::protocol::client::Event;
use quecto_tui::shell::app::tui_harness::TuiHarness;

/// The token rendered in both the direct-path baseline and the feed frame.
const SEAM_TOKEN: &str = "seam-parity-token";

fn harness(world: &mut TuiWorld) -> &mut TuiHarness {
    &mut world.tui_parity.as_mut().expect("TUI harness").0
}

#[given("a baseline frame from a master token handled directly")]
fn baseline_frame_from_direct_handling(world: &mut TuiWorld) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut baseline = rt.block_on(TuiHarness::new());
    baseline.event(Event::Token {
        token: SEAM_TOKEN.into(),
    });
    let frame = baseline.full_frame();
    assert!(
        frame.contains(SEAM_TOKEN),
        "precondition: direct handling renders the token"
    );
    world.tui_seam_baseline_frame = Some(frame);
    world.tui_parity_rt = Some(rt);
}

#[given("a fresh headless TUI harness")]
fn fresh_headless_harness(world: &mut TuiWorld) {
    if world.tui_parity_rt.is_none() {
        world.tui_parity_rt = Some(tokio::runtime::Runtime::new().expect("tokio runtime"));
    }
    let rt = world.tui_parity_rt.as_ref().expect("runtime");
    let h = rt.block_on(TuiHarness::new());
    world.tui_parity = Some(TuiParityHarness(h));
}

#[when("the same master token arrives through the master connection feed")]
fn token_arrives_through_connection_feed(world: &mut TuiWorld) {
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("runtime")
        .handle()
        .clone();
    let h = harness(world);
    handle.block_on(async {
        // Written on the agent side of the REAL socket, so the token flows
        // through the whole production path: client reader → connection feed
        // task → shared fan-in → sourced routing.
        h.wire_master_event_line(&format!(r#"{{"type":"token","token":"{SEAM_TOKEN}"}}"#))
            .await;
    });
}

#[then("the frame should be identical to the direct-handling baseline")]
fn frame_identical_to_baseline(world: &mut TuiWorld) {
    let expected = world
        .tui_seam_baseline_frame
        .take()
        .expect("baseline frame captured in the Given");
    let got = harness(world).full_frame();
    assert!(
        got.contains(SEAM_TOKEN),
        "a master event delivered via the connection feed must reach the master session's chat (#1462), got:\n{got}"
    );
    assert_eq!(
        got, expected,
        "N=1 frames must be byte-identical between the direct path and the connection feed path (#1462)"
    );
}

#[when("the master connection's event stream closes")]
fn master_event_stream_closes(world: &mut TuiWorld) {
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("runtime")
        .handle()
        .clone();
    let h = harness(world);
    handle.block_on(async {
        // Real EOF on the agent side; the feed task turns it into the
        // Source::Closed sentinel the routing drains.
        h.wire_close_master_connection().await;
    });
}

#[when("the agent child process aborts")]
fn agent_child_process_aborts(world: &mut TuiWorld) {
    let watch = world
        .tui_disconnect_child
        .take()
        .expect("spawned agent child");
    // Abort the REAL child with a real SIGABRT (same technique as #1047);
    // kill with a valid signal returns an error code rather than faulting.
    // SAFETY: pid comes from a child we just spawned.
    let rc = unsafe { libc::kill(watch.pid as i32, libc::SIGABRT) };
    assert_eq!(rc, 0, "SIGABRT must be delivered to the spawned child");
    // Attach the production exit watcher so the subsequent stream close can
    // diagnose WHY the agent went away (#1047).
    harness(world).app_mut().set_child_exit_watch(watch.watch);
}
