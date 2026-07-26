//! Regression tests for the below-chat working indicator spacing.

use super::tui_harness::TuiHarness;

async fn harness() -> TuiHarness {
    TuiHarness::new().await
}

#[tokio::test]
async fn compose_bottom_spaces_active_child_indicator() {
    let mut h = harness().await;
    h.event(super::tui_harness::subagents_changed(vec![
        super::tui_harness::subagent("w1", "running", None),
    ]));

    let bottom = h.app_mut().compose_bottom(120);
    let joined = bottom.join("\n");
    let activity = bottom
        .iter()
        .position(|line| super::app_methods::strip_ansi(line).contains("working"))
        .unwrap_or_else(|| panic!("expected activity line in bottom stack: {joined}"));
    assert!(
        activity > 0,
        "activity line must have a spacer above it: {joined}"
    );
    assert_eq!(
        super::app_methods::strip_ansi(&bottom[activity - 1]).trim(),
        "",
        "activity line must have exactly one blank spacer immediately above it: {joined}"
    );
}

#[tokio::test]
async fn compose_bottom_removes_idle_reserved_gap_above_input() {
    let mut h = harness().await;
    h.event(super::tui_harness::subagents_changed(vec![
        super::tui_harness::subagent("w1", "idle", None),
    ]));

    let bottom = h.app_mut().compose_bottom(120);
    assert!(
        bottom
            .first()
            .is_some_and(|line| !super::app_methods::strip_ansi(line).trim().is_empty()),
        "idle tracked subagents must not reserve a blank gap above the input: {bottom:?}"
    );
}
