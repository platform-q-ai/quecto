//! Regression tests for the below-chat working indicator spacing.

use super::tui_harness::TuiHarness;

async fn harness() -> TuiHarness {
    TuiHarness::new().await
}

#[tokio::test]
async fn compose_bottom_shows_active_child_indicator_in_stable_slot() {
    let mut h = harness().await;
    h.event(super::tui_harness::subagents_changed(vec![
        super::tui_harness::subagent("w1", "running", None),
    ]));

    let bottom = h.app_mut().compose_bottom(120);
    let joined = bottom.join("\n");
    assert!(
        bottom.first().is_some_and(
            |line| super::app_render_helpers::strip_ansi(line).contains("subagent working")
        ),
        "active child indicator must render in the stable tracked-child slot: {joined}"
    );
}

#[tokio::test]
async fn compose_bottom_keeps_tracked_child_activity_slot_stable() {
    let mut idle = harness().await;
    idle.event(super::tui_harness::subagents_changed(vec![
        super::tui_harness::subagent("w1", "idle", None),
    ]));
    let idle_bottom = idle.app_mut().compose_bottom(120);

    let mut active = harness().await;
    active.event(super::tui_harness::subagents_changed(vec![
        super::tui_harness::subagent("w1", "running", None),
    ]));
    let active_bottom = active.app_mut().compose_bottom(120);

    assert!(
        idle_bottom.first().is_some_and(
            |line| super::app_render_helpers::strip_ansi(line).contains("subagent idle")
        ),
        "idle tracked subagents should reserve the activity slot with visible status, not a blank gap: {idle_bottom:?}"
    );
    assert!(
        active_bottom.first().is_some_and(
            |line| super::app_render_helpers::strip_ansi(line).contains("subagent working")
        ),
        "active tracked subagents should reuse the reserved activity slot: {active_bottom:?}"
    );
    assert_eq!(
        idle_bottom.len(),
        active_bottom.len(),
        "tracked child idle↔active transitions must not resize the bottom stack"
    );
}
