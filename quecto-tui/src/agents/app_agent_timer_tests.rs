use super::app_subagents_tests::{harness, info};
use super::tui_harness::{TuiHarness, subagent, subagents_changed};
use crate::protocol::client::Event;
use crate::shell::keys::Key;

#[tokio::test(start_paused = true)]
async fn idle_subagent_main_pane_title_does_not_repeat_idle() {
    let mut h = harness().await;
    h.app_mut()
        .update_subagent_bar(vec![info("worker", "running")]);
    tokio::time::advance(std::time::Duration::from_secs(12)).await;
    h.app_mut()
        .update_subagent_bar(vec![info("worker", "idle")]);
    h.app_mut().select_agent(Some("worker"));

    let pane = h.main_pane();
    let title_line = pane
        .lines()
        .find(|line| line.contains("worker") && line.contains("idle"))
        .unwrap_or_else(|| panic!("selected idle sub-agent title not found:\n{pane}"));
    assert!(
        title_line.contains("worker · idle (ran 0:12"),
        "idle status should be followed by elapsed run duration, not a second idle: {title_line:?}"
    );
    assert!(
        !title_line.contains("idle idle"),
        "idle status must not render twice in main pane title: {title_line:?}"
    );
}

/// The idle Coordinator row must not display a wall-clock uptime that advances only
/// when incidental input causes a repaint. It should show the frozen duration of
/// the last active run, matching idle sub-agent timer semantics.
#[tokio::test(start_paused = true)]
async fn idle_coordinator_panel_timer_is_frozen_across_advancing_now() {
    let mut h = TuiHarness::new().await;
    let now1 = tokio::time::Instant::now();
    let now2 = now1 + std::time::Duration::from_secs(60);

    let v1 = h.app_mut().panel_row_elapsed(None, now1);
    let v2 = h.app_mut().panel_row_elapsed(None, now2);

    assert_eq!(
        v1, v2,
        "idle coordinator timer must be frozen, not advance on incidental renders: \
         {v1:?} vs {v2:?}"
    );
    assert_eq!(v1, "0:00", "a never-run idle coordinator should show 0:00");
}

/// Coordinator elapsed time is cumulative active processing time for this
/// connection/session. A later message or wake resumes the frozen value rather
/// than restarting the visible counter at zero (#1726).
#[tokio::test(start_paused = true)]
async fn coordinator_panel_timer_accumulates_across_message_boundaries() {
    let mut h = TuiHarness::new().await;

    h.event(Event::AgentStart);
    tokio::time::advance(std::time::Duration::from_secs(30)).await;
    h.event(Event::AgentEnd {
        messages: vec![],
        message_refs: vec![],
    });
    tokio::time::advance(std::time::Duration::from_secs(90)).await;

    assert_eq!(
        h.app_mut()
            .panel_row_elapsed(None, tokio::time::Instant::now()),
        "0:30",
        "idle time between messages must not count"
    );

    h.event(Event::AgentStart);
    assert_eq!(
        h.app_mut()
            .panel_row_elapsed(None, tokio::time::Instant::now()),
        "0:30",
        "a new message or wake must resume the session counter"
    );
    tokio::time::advance(std::time::Duration::from_secs(15)).await;

    assert_eq!(
        h.app_mut()
            .panel_row_elapsed(None, tokio::time::Instant::now()),
        "0:45",
        "only active processing time should accumulate"
    );
}

#[tokio::test(start_paused = true)]
async fn coordinator_disconnect_after_end_does_not_count_idle_gap() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    tokio::time::advance(std::time::Duration::from_secs(10)).await;
    h.event(Event::AgentEnd {
        messages: vec![],
        message_refs: vec![],
    });
    tokio::time::advance(std::time::Duration::from_secs(60)).await;

    h.app_mut().handle_agent_disconnected(None);
    h.event(Event::AgentStart);

    assert_eq!(
        h.app_mut()
            .panel_row_elapsed(None, tokio::time::Instant::now()),
        "0:10",
        "a late disconnect while already stopped must not incorporate idle time"
    );
}

#[tokio::test(start_paused = true)]
async fn coordinator_abort_then_stale_end_does_not_count_idle_gap() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    tokio::time::advance(std::time::Duration::from_secs(10)).await;
    h.app_mut().handle_key(Key::Escape);
    tokio::time::advance(std::time::Duration::from_secs(60)).await;
    h.event(Event::AgentEnd {
        messages: vec![],
        message_refs: vec![],
    });
    h.event(Event::AgentStart);

    assert_eq!(
        h.app_mut()
            .panel_row_elapsed(None, tokio::time::Instant::now()),
        "0:10",
        "a stale end after abort must not incorporate idle time"
    );
}

#[tokio::test(start_paused = true)]
async fn coordinator_panel_timer_resets_for_new_session() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    tokio::time::advance(std::time::Duration::from_secs(30)).await;
    h.event(Event::AgentEnd {
        messages: vec![],
        message_refs: vec![],
    });

    h.app_mut().reset_session("New session started");

    assert_eq!(
        h.app_mut()
            .panel_row_elapsed(None, tokio::time::Instant::now()),
        "0:00",
        "an explicit new-session boundary must reset cumulative runtime"
    );
}

/// While the Coordinator is actively running, its timer must still advance so the
/// TUI can repaint it on the existing active-turn animation tick.
#[tokio::test]
async fn running_coordinator_panel_timer_still_advances_with_now() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    let now1 = tokio::time::Instant::now();
    let now2 = now1 + std::time::Duration::from_secs(60);

    let v1 = h.app_mut().panel_row_elapsed(None, now1);
    let v2 = h.app_mut().panel_row_elapsed(None, now2);

    assert_ne!(
        v1, v2,
        "a running coordinator timer must keep tracking now: {v1:?} vs {v2:?}"
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

/// A sub-agent elapsed timer should pause while idle and resume from the
/// previous elapsed duration when it starts running again.
#[tokio::test(start_paused = true)]
async fn subagent_panel_timer_accumulates_running_time_across_idle_gap() {
    let mut h = harness().await;
    h.app_mut()
        .update_subagent_bar(vec![info("worker", "running")]);
    tokio::time::advance(std::time::Duration::from_secs(30)).await;

    h.app_mut()
        .update_subagent_bar(vec![info("worker", "idle")]);
    tokio::time::advance(std::time::Duration::from_secs(20)).await;
    let while_idle = h
        .app_mut()
        .panel_row_elapsed(Some("worker"), tokio::time::Instant::now());
    assert_eq!(
        while_idle, "idle (ran 0:30)",
        "idle gap must not advance elapsed time"
    );

    h.app_mut()
        .update_subagent_bar(vec![info("worker", "running")]);
    tokio::time::advance(std::time::Duration::from_secs(5)).await;
    let after_resume = h
        .app_mut()
        .panel_row_elapsed(Some("worker"), tokio::time::Instant::now());

    assert_eq!(
        after_resume, "0:35",
        "timer should resume from prior running time instead of resetting"
    );
}

/// Optimistic spawn rows begin as `starting`; the successful spawn result must
/// also start their elapsed timer when it flips them to `running`.
#[tokio::test(start_paused = true)]
async fn spawn_result_starts_optimistic_subagent_timer() {
    let mut h = TuiHarness::new().await;
    h.app_mut().handle_event(Event::ToolExecutionStart {
        tool_call_id: "spawn-1".into(),
        tool_name: "spawn".into(),
        args: serde_json::json!({"agent_id": "worker"}),
    });

    tokio::time::advance(std::time::Duration::from_secs(20)).await;
    assert_eq!(
        h.app_mut()
            .panel_row_elapsed(Some("worker"), tokio::time::Instant::now()),
        "0:00",
        "time spent waiting for spawn result must not count as run time"
    );

    h.app_mut().handle_event(Event::ToolExecutionEnd {
        tool_call_id: "spawn-1".into(),
        tool_name: "spawn".into(),
        result: serde_json::json!({
            "content": [{"type": "text", "text": "Subagent 'worker' is running"}]
        }),
        is_error: false,
    });
    tokio::time::advance(std::time::Duration::from_secs(5)).await;

    assert_eq!(
        h.app_mut()
            .panel_row_elapsed(Some("worker"), tokio::time::Instant::now()),
        "0:05",
        "timer should run from the successful spawn result"
    );
}

#[tokio::test(start_paused = true)]
async fn esc_cancel_running_subagent_freezes_parent_roster_timer_and_working_bar() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![
        subagent("worker", "running", Some(("active", 1, 3))),
        subagent("other", "idle", Some(("active", 2, 3))),
    ]));
    h.app_mut().select_agent(Some("worker"));
    tokio::time::advance(std::time::Duration::from_secs(10)).await;

    h.app_mut().handle_key(Key::Escape);

    assert!(
        !h.app_mut().active_subagent_running(),
        "Esc abort should immediately make the selected sub-agent non-running"
    );
    assert_eq!(
        h.app_mut()
            .panel_row_elapsed(Some("worker"), tokio::time::Instant::now()),
        "idle (ran 0:10)",
        "parent roster row should freeze when Esc interrupts the sub-agent"
    );
    assert_eq!(
        h.app_mut().ac().roster.tracked_active_count(),
        0,
        "Esc should clear the parent roster's active status so the working bar stops"
    );

    tokio::time::advance(std::time::Duration::from_secs(20)).await;
    assert_eq!(
        h.app_mut()
            .panel_row_elapsed(Some("worker"), tokio::time::Instant::now()),
        "idle (ran 0:10)",
        "interrupted sub-agent timer must stay frozen while no running update arrives"
    );
}

fn resume_answer(
    h: &mut TuiHarness,
    id: Option<&str>,
    success: bool,
    data: Option<serde_json::Value>,
) {
    if let Some(id) = id {
        h.app_mut().test_arm_resume_session(id);
    }
    h.app_mut().handle_response(
        id.map(String::from),
        "resume_session".into(),
        success,
        data,
        (!success).then(|| "err".to_string()),
    );
}

fn end_event() -> Event {
    Event::AgentEnd {
        messages: vec![],
        message_refs: vec![],
    }
}

/// The clock starts at zero on the first turn and a repeated start is a
/// no-op: time before the first prompt, and the start itself, never count.
#[tokio::test(start_paused = true)]
async fn coordinator_clock_starts_at_zero_and_ignores_repeated_starts() {
    let mut h = TuiHarness::new().await;
    tokio::time::advance(std::time::Duration::from_secs(20)).await;
    assert_eq!(
        h.app_mut()
            .panel_row_elapsed(None, tokio::time::Instant::now()),
        "0:00"
    );
    h.event(Event::AgentStart);
    tokio::time::advance(std::time::Duration::from_secs(5)).await;
    h.event(Event::AgentStart);
    tokio::time::advance(std::time::Duration::from_secs(5)).await;
    assert_eq!(
        h.app_mut()
            .panel_row_elapsed(None, tokio::time::Instant::now()),
        "0:10",
        "pre-prompt time is excluded and a second start keeps the origin"
    );
}

#[tokio::test(start_paused = true)]
async fn late_agent_error_does_not_add_idle_time_to_coordinator_timer() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    tokio::time::advance(std::time::Duration::from_secs(10)).await;
    h.event(end_event());
    tokio::time::advance(std::time::Duration::from_secs(60)).await;
    h.app_mut()
        .handle_response(None, "agent_error".into(), false, None, Some("late".into()));
    h.event(Event::AgentStart);
    assert_eq!(
        h.app_mut()
            .panel_row_elapsed(None, tokio::time::Instant::now()),
        "0:10",
        "a late error while stopped must not incorporate idle time"
    );
}

/// A resume answer is a session boundary only when it names a different
/// session (or one this tab had not learned yet); a failed resume and a
/// resume into the same session keep the clock.
#[tokio::test(start_paused = true)]
async fn resume_resets_the_clock_only_on_a_session_identity_change() {
    let mut h = TuiHarness::new().await;
    h.app_mut().ac_mut().session_key = Some("cli:original".into());
    h.event(Event::AgentStart);
    tokio::time::advance(std::time::Duration::from_secs(10)).await;
    h.event(end_event());

    resume_answer(&mut h, Some("resume-1"), false, None);
    assert_eq!(
        h.app_mut()
            .panel_row_elapsed(None, tokio::time::Instant::now()),
        "0:10",
        "failed resume must preserve current-session runtime"
    );
    let same = serde_json::json!({"session": "original", "sessionKey": "cli:original"});
    resume_answer(&mut h, Some("resume-2"), true, Some(same));
    assert_eq!(
        h.app_mut()
            .panel_row_elapsed(None, tokio::time::Instant::now()),
        "0:10"
    );
    let other = serde_json::json!({"session": "alpha", "sessionKey": "cli:alpha"});
    resume_answer(&mut h, Some("resume-3"), true, Some(other));
    assert_eq!(
        h.app_mut()
            .panel_row_elapsed(None, tokio::time::Instant::now()),
        "0:00",
        "successful resume must reset runtime for the new session identity"
    );

    // A tab that has not learned its session yet treats the first resumed
    // identity as a boundary too.
    let mut fresh = TuiHarness::new().await;
    fresh.event(Event::AgentStart);
    tokio::time::advance(std::time::Duration::from_secs(10)).await;
    fresh.event(end_event());
    resume_answer(
        &mut fresh,
        Some("resume-4"),
        true,
        Some(serde_json::json!({"session": "beta", "sessionKey": "cli:beta"})),
    );
    assert_eq!(
        fresh
            .app_mut()
            .panel_row_elapsed(None, tokio::time::Instant::now()),
        "0:00"
    );
}

/// Only this tab's own resume answer settles its resume latches: a foreign
/// answer (another client resumed the shared agent) still refreshes the view
/// but leaves the in-flight resume to be answered.
#[tokio::test(start_paused = true)]
async fn only_the_owned_resume_answer_settles_the_latches() {
    let mut h = TuiHarness::new().await;
    h.app_mut().ac_mut().pending_session_resume = Some("cli:mine".into());
    h.app_mut().test_arm_resume_session("resume-mine");
    h.app_mut().handle_response(
        Some("resume-other".into()),
        "resume_session".into(),
        true,
        Some(serde_json::json!({"session": "theirs", "sessionKey": "cli:theirs"})),
        None,
    );
    assert!(
        h.app_mut().test_pending_resume_messages_id().is_some(),
        "a foreign resume answer still reloads the transcript"
    );
    assert_eq!(
        h.app_mut().test_pending_session_resume(),
        (Some("cli:mine"), Some("resume-mine")),
        "a foreign answer leaves this tab's resume in flight"
    );
    h.app_mut().handle_response(
        Some("resume-mine".into()),
        "resume_session".into(),
        false,
        None,
        Some("session not found".into()),
    );
    assert_eq!(
        h.app_mut().test_pending_session_resume(),
        (None, None),
        "an owned failure clears the deferred session so a reattach cannot replay it"
    );
    assert!(!h.app_mut().notifications.is_empty());
}
