use super::app_subagents_tests::{harness, info};
use super::tui_harness::TuiHarness;
use crate::protocol::client::Event;

/// The idle Master row must not display a wall-clock uptime that advances only
/// when incidental input causes a repaint. It should show the frozen duration of
/// the last active run, matching idle sub-agent timer semantics.
#[tokio::test(start_paused = true)]
async fn idle_master_panel_timer_is_frozen_across_advancing_now() {
    let mut h = TuiHarness::new().await;
    let now1 = tokio::time::Instant::now();
    let now2 = now1 + std::time::Duration::from_secs(60);

    let v1 = h.app_mut().panel_row_elapsed(None, now1);
    let v2 = h.app_mut().panel_row_elapsed(None, now2);

    assert_eq!(
        v1, v2,
        "idle master timer must be frozen, not advance on incidental renders: \
         {v1:?} vs {v2:?}"
    );
    assert_eq!(v1, "0:00", "a never-run idle master should show 0:00");
}

/// While the Master is actively running, its timer must still advance so the
/// TUI can repaint it on the existing active-turn animation tick.
#[tokio::test]
async fn running_master_panel_timer_still_advances_with_now() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    let now1 = tokio::time::Instant::now();
    let now2 = now1 + std::time::Duration::from_secs(60);

    let v1 = h.app_mut().panel_row_elapsed(None, now1);
    let v2 = h.app_mut().panel_row_elapsed(None, now2);

    assert_ne!(
        v1, v2,
        "a running master timer must keep tracking now: {v1:?} vs {v2:?}"
    );
}

/// A sub-agent elapsed timer is run-time, not lifetime: statuses before the
/// worker is actually running (for example `starting`) must not accumulate time.
#[tokio::test(start_paused = true)]
async fn starting_subagent_panel_timer_is_frozen_until_running() {
    let mut h = harness().await;
    h.app_mut()
        .update_subagent_bar(vec![info("worker", "starting")]);

    tokio::time::advance(std::time::Duration::from_secs(30)).await;
    let before_running = h
        .app_mut()
        .panel_row_elapsed(Some("worker"), tokio::time::Instant::now());

    assert_eq!(
        before_running, "0:00",
        "starting time must not count as agent run time"
    );

    h.app_mut()
        .update_subagent_bar(vec![info("worker", "running")]);
    tokio::time::advance(std::time::Duration::from_secs(45)).await;
    let after_running = h
        .app_mut()
        .panel_row_elapsed(Some("worker"), tokio::time::Instant::now());

    assert_eq!(
        after_running, "0:45",
        "timer should start from the transition into running, not from first observation"
    );
}
