//! Tests for idle event-loop scheduling and animation/fallback servicing (#978).

use super::tui_harness::TuiHarness;
use super::*;

async fn harness() -> TuiHarness {
    TuiHarness::new().await
}
#[tokio::test]
async fn quiet_idle_session_does_not_need_animation_tick_after_kitty_fallback() {
    let mut h = harness().await;
    let app = h.app_mut();

    assert!(
        !app.needs_animation_tick(false),
        "idle TUI with no animation, notifications, subagents, streaming, or fallback should not arm a sub-second timer"
    );
}

#[tokio::test]
async fn pending_kitty_fallback_keeps_animation_tick_armed() {
    let mut h = harness().await;
    let app = h.app_mut();

    assert!(
        app.needs_animation_tick(true),
        "unsupported-terminal fallback deadline must still be serviced"
    );
}

#[tokio::test]
async fn visible_animation_keeps_animation_tick_armed() {
    let mut h = harness().await;
    let app = h.app_mut();

    app.spinner = Some(Spinner::new("working"));
    assert!(
        app.needs_animation_tick(false),
        "a visible spinner must continue advancing while the TUI is otherwise idle"
    );

    app.spinner = None;
    app.notify("saved", NotifyLevel::Info);
    assert!(
        app.needs_animation_tick(false),
        "active notifications must keep their expiry animation deadline serviced"
    );

    app.notifications.gc();
    app.agent_state.start();
    assert!(
        app.needs_animation_tick(false),
        "running master work must continue advancing the activity indicator"
    );

    app.agent_state.end();
    app.master_session.footer.set_streaming(true);
    assert!(
        app.needs_animation_tick(false),
        "streaming status must continue advancing the activity indicator"
    );
}

#[tokio::test]
async fn spinner_animation_tick_advances_visible_frame() {
    let mut h = harness().await;
    let app = h.app_mut();
    app.spinner = Some(Spinner::new("working"));
    let before = app.spinner.as_ref().unwrap().frame_index();
    let mut fallback_done = true;

    assert!(app.service_animation_tick(&mut fallback_done, tokio::time::Instant::now()));

    assert_ne!(
        app.spinner.as_ref().unwrap().frame_index(),
        before,
        "spinner service tick should visibly advance the spinner frame"
    );
}

#[tokio::test]
async fn notification_animation_tick_keeps_visible_notification_serviced() {
    let mut h = harness().await;
    let app = h.app_mut();
    app.notify("saved", NotifyLevel::Info);
    let mut fallback_done = true;

    assert!(app.needs_animation_tick(false));
    assert!(!app.service_animation_tick(&mut fallback_done, tokio::time::Instant::now()));
    assert!(
        !app.notifications.is_empty(),
        "fresh notification should remain visible after being serviced"
    );
}

#[tokio::test]
async fn active_subagent_keeps_animation_tick_armed() {
    let mut h = harness().await;
    h.event(super::tui_harness::subagents_changed(vec![
        super::tui_harness::subagent("worker", "running", None),
    ]));
    let app = h.app_mut();

    assert!(
        app.needs_animation_tick(false),
        "an active subagent must keep its elapsed-time and spinner animation ticking"
    );
}

#[tokio::test(start_paused = true)]
async fn kitty_fallback_waits_for_deadline_then_enables_modify_other_keys() {
    let mut h = harness().await;
    let app = h.app_mut();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(150);
    let mut fallback_done = false;

    tokio::time::advance(Duration::from_millis(149)).await;
    app.service_animation_tick(&mut fallback_done, deadline);
    assert!(
        !fallback_done,
        "fallback must not complete before its deadline"
    );
    assert!(
        !app.kitty.modify_other_keys,
        "fallback keyboard mode must not be enabled before the deadline"
    );

    tokio::time::advance(Duration::from_millis(1)).await;
    app.service_animation_tick(&mut fallback_done, deadline);
    assert!(fallback_done, "fallback completes at the deadline");
    assert!(
        app.kitty.modify_other_keys,
        "unsupported terminals must get modifyOtherKeys fallback after the deadline"
    );
    assert!(
        !app.needs_animation_tick(false),
        "kitty fallback alone must not keep a sub-second timer armed after completion"
    );
}

#[tokio::test(start_paused = true)]
async fn kitty_response_before_deadline_cancels_fallback() {
    let mut h = harness().await;
    let app = h.app_mut();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(150);
    let mut fallback_done = false;

    tokio::time::advance(Duration::from_millis(149)).await;
    assert!(
        !app.process_stdin_bytes(
            b"\x1b[?7u".to_vec(),
            &mut tokio::sync::mpsc::channel(1).1,
            Duration::from_millis(10),
            &mut fallback_done,
        )
        .await
    );
    assert!(fallback_done, "kitty response completes protocol detection");
    assert!(app.kitty.active, "kitty protocol is enabled on a response");

    tokio::time::advance(Duration::from_millis(1)).await;
    app.service_animation_tick(&mut fallback_done, deadline);
    assert!(
        !app.kitty.modify_other_keys,
        "fallback must not be enabled after a confirmed kitty response"
    );
}
