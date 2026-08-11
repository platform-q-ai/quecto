//! Step definitions for `tui_master_connection_feed.feature` (#1462).
//!
//! Phase 1 of the multi-session TUI (epic #1467): the master connection moves
//! behind a feed task and its events arrive through the shared fan-in channel
//! keyed by `Source`. At N=1 nothing may change: these steps pin frame parity
//! between the direct path and the fan-in path, and that stream close — now an
//! explicit `Source::Closed` sentinel — keeps the #1047 disconnect diagnosis.

use super::*;
use quecto_tui::protocol::client::Event;
use quecto_tui::shell::app::tui_harness::TuiHarness;

/// The token rendered in both the direct-path baseline and the fan-in frame.
const SEAM_TOKEN: &str = "seam-parity-token";

fn harness(world: &mut TuiWorld) -> &mut TuiHarness {
    &mut world.tui_parity.as_mut().expect("TUI harness").0
}

#[given("a headless TUI harness showing a master token via direct handling")]
fn direct_handling_baseline(world: &mut TuiWorld) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    // Baseline: the pre-seam direct path renders the token.
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
    // Fresh harness for the fan-in delivery under test.
    let fan_in = rt.block_on(TuiHarness::new());
    world.tui_parity_rt = Some(rt);
    world.tui_parity = Some(TuiParityHarness(fan_in));
}

#[when("the same master token is delivered through the fan-in tagged with the master tab source")]
fn deliver_token_via_fan_in(world: &mut TuiWorld) {
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("runtime")
        .handle()
        .clone();
    let h = harness(world);
    handle.block_on(async {
        h.sourced_master_event(Event::Token {
            token: SEAM_TOKEN.into(),
        })
        .await;
    });
}

#[then("the fan-in frame should be identical to the directly handled frame")]
fn fan_in_frame_identical(world: &mut TuiWorld) {
    let expected = world
        .tui_seam_baseline_frame
        .take()
        .expect("baseline frame captured in the Given");
    let got = harness(world).full_frame();
    assert!(
        got.contains(SEAM_TOKEN),
        "a Source::Tab(MASTER) event must reach the master session's chat (#1462), got:\n{got}"
    );
    assert_eq!(
        got, expected,
        "N=1 frames must be byte-identical between the direct path and the fan-in path (#1462)"
    );
}

#[when("the master connection delivers its Closed sentinel")]
fn master_connection_delivers_closed_sentinel(world: &mut TuiWorld) {
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("runtime")
        .handle()
        .clone();
    let h = harness(world);
    handle.block_on(async {
        h.deliver_closed_sentinel().await;
    });
}

#[when("the agent child process aborts and the Closed sentinel is delivered")]
fn child_aborts_then_closed_sentinel(world: &mut TuiWorld) {
    let watch = world
        .tui_disconnect_child
        .take()
        .expect("spawned agent child");
    // Abort the REAL child with a real SIGABRT (same technique as #1047);
    // kill with a valid signal returns an error code rather than faulting.
    // SAFETY: pid comes from a child we just spawned.
    let rc = unsafe { libc::kill(watch.pid as i32, libc::SIGABRT) };
    assert_eq!(rc, 0, "SIGABRT must be delivered to the spawned child");

    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("runtime")
        .handle()
        .clone();
    let h = &mut world.tui_parity.as_mut().expect("TUI harness").0;
    handle.block_on(async {
        h.deliver_closed_sentinel_with_child_watch(watch.watch)
            .await;
    });
}
