use super::*;
use crate::protocol::client::Event;
use crate::shell::app::app_events::app_events_test_support::test_app;
use crate::shell::app::tui_harness::{subagent_with_socket, subagents_changed};

fn waiting(seconds: u64) -> serde_json::Value {
    serde_json::json!({
        "waiting": 1, "admitted": 0, "longestWaitSeconds": seconds, "revision": 1,
        "groups": [{"group": "anthropic"}]
    })
}

fn cooldown(group: &str, seconds: u64) -> serde_json::Value {
    serde_json::json!({
        "waiting": 0, "revision": 3,
        "groups": [{"group": group, "cooldown": {"state": "until", "remainingSeconds": seconds}}]
    })
}

fn event(agent_id: Option<&str>, admission: serde_json::Value) -> Event {
    Event::AdmissionStateChanged {
        agent_id: agent_id.map(str::to_string),
        admission,
    }
}

fn agent_end() -> Event {
    Event::AgentEnd {
        messages: vec![],
        message_refs: vec![],
    }
}

#[tokio::test]
async fn the_master_view_paints_the_footer_and_spinner_without_changing_state() {
    let mut app = test_app().await;
    app.handle_event(Event::AgentStart);
    assert!(app.ac().master_session.running);
    app.handle_event(event(None, waiting(12)));
    assert!(
        app.ac().master_session.running,
        "admission is not a lifecycle state"
    );
    let footer = &app.ac().master_session.footer;
    assert_eq!(footer.admission(), Some("12s"));
    assert_eq!(footer.admission_compact(), Some("12s"));
    assert_eq!(
        app.ac().spinner.as_ref().unwrap().message(),
        "⏳ 12s (Esc to interrupt)"
    );
    // Granted: the label leaves and the spinner returns to its plain message.
    app.handle_event(event(
        Some(""),
        serde_json::json!({"waiting": 0, "admitted": 1, "revision": 2}),
    ));
    assert_eq!(app.ac().master_session.footer.admission(), None);
    assert_eq!(
        app.ac().spinner.as_ref().unwrap().message(),
        WORKING_MESSAGE
    );
    // A cooldown-only view survives the end of the run; a waiting one does not.
    app.handle_event(event(None, cooldown("anthropic", 30)));
    app.handle_event(agent_end());
    assert_eq!(
        app.ac().master_session.footer.admission(),
        Some("anthropic cooldown 30s")
    );
    app.handle_event(Event::AgentStart);
    app.handle_event(event(None, waiting(1)));
    app.handle_event(agent_end());
    assert_eq!(app.ac().master_session.footer.admission(), None);
    // A cooldown-only label whose group happens to be called "waiting…"
    // survives the run end: the decision is made on the view, not the text.
    app.handle_event(event(None, cooldown("waiting-room", 9)));
    app.handle_event(agent_end());
    assert_eq!(
        app.ac().master_session.footer.admission(),
        Some("waiting-room cooldown 9s")
    );
    // Garbage is ignored.
    app.handle_event(event(None, serde_json::json!("nope")));
    assert_eq!(
        app.ac().master_session.footer.admission(),
        Some("waiting-room cooldown 9s")
    );
}

#[tokio::test]
async fn a_tool_spinner_message_is_never_clobbered_and_disconnect_clears_the_label() {
    let mut app = test_app().await;
    app.handle_event(Event::AgentStart);
    app.handle_event(event(None, waiting(2)));
    // A tool takes over the spinner while the wait ends.
    app.ac_mut()
        .spinner
        .as_mut()
        .unwrap()
        .set_message("Spawning reviewer...");
    app.handle_event(event(
        None,
        serde_json::json!({"waiting": 0, "revision": 2}),
    ));
    assert_eq!(
        app.ac().spinner.as_ref().unwrap().message(),
        "Spawning reviewer...",
        "a message this module did not write is left alone"
    );
    // A view without any wait never touches the spinner at all.
    app.handle_event(event(None, cooldown("anthropic", 5)));
    assert_eq!(
        app.ac().spinner.as_ref().unwrap().message(),
        "Spawning reviewer..."
    );
    // Disconnect drops the view and the label.
    app.mark_agent_disconnected_for_test();
    assert_eq!(app.ac().master_session.footer.admission(), None);
    assert!(app.ac().admission_view.is_none());
}

#[tokio::test]
async fn a_forwarded_child_view_labels_a_tracked_child_only() {
    let mut app = test_app().await;
    // Unknown ids never grow the map.
    app.handle_event(event(Some("stranger"), waiting(4)));
    assert!(app.ac().roster.admission_labels.is_empty());
    app.handle_event(subagents_changed(vec![subagent_with_socket(
        "reviewer", "running", None, None,
    )]));
    app.handle_event(event(Some("reviewer"), waiting(4)));
    assert_eq!(app.ac().master_session.footer.admission(), None);
    assert_eq!(
        app.ac()
            .roster
            .admission_labels
            .get("reviewer")
            .map(String::as_str),
        Some("4s")
    );
    // The child leaves the roster: the next update prunes its label.
    app.handle_event(subagents_changed(vec![]));
    app.handle_event(event(Some("someone-else"), waiting(1)));
    assert!(app.ac().roster.admission_labels.is_empty(), "pruned");
    app.handle_event(subagents_changed(vec![subagent_with_socket(
        "reviewer", "running", None, None,
    )]));
    app.handle_event(event(Some("reviewer"), waiting(4)));
    app.handle_event(event(
        Some("reviewer"),
        serde_json::json!({"waiting": 0, "revision": 2}),
    ));
    assert!(app.ac().roster.admission_labels.is_empty());
    // The connected agent's own id addresses the master.
    app.ac_mut().connected_agent_id = Some("me".into());
    app.handle_event(event(Some("me"), waiting(2)));
    assert_eq!(app.ac().master_session.footer.admission(), Some("2s"));
}

#[tokio::test]
async fn get_state_applies_or_clears_the_admission_view() {
    let mut app = test_app().await;
    let response = |admission: Option<serde_json::Value>| {
        let mut data = serde_json::json!({
            "state": "thinking", "model": "m", "sessionKey": "cli:default",
            "progress": {"state": "waiting", "reason": "x"}, "generation": 3
        });
        if let Some(admission) = admission {
            data["admission"] = admission;
        }
        Event::Response {
            id: None,
            command: "get_state".into(),
            success: true,
            data: Some(data),
            error: None,
        }
    };
    app.handle_event(response(Some(waiting(7))));
    assert_eq!(app.ac().master_session.footer.admission(), Some("7s"));
    // An agent without an authority reports no view: the label is cleared.
    app.handle_event(response(None));
    assert_eq!(app.ac().master_session.footer.admission(), None);
    assert_eq!(capitalize(""), "");
    assert_eq!(capitalize("éa"), "Éa");
}

#[tokio::test]
async fn agent_error_ends_the_wait_and_the_panel_row_paints_a_child_label() {
    let mut app = test_app().await;
    app.handle_event(Event::AgentStart);
    app.handle_event(event(None, waiting(5)));
    app.handle_event(Event::Response {
        id: None,
        command: "agent_error".into(),
        success: false,
        data: None,
        error: Some("boom".into()),
    });
    assert!(!app.ac().master_session.running);
    assert_eq!(app.ac().master_session.footer.admission(), None);
    // A tool ending while the master is queued keeps the wait on the spinner.
    app.handle_event(Event::AgentStart);
    app.handle_event(event(None, waiting(6)));
    app.handle_event(Event::ToolExecutionEnd {
        tool_call_id: "c1".into(),
        tool_name: "bash".into(),
        result: serde_json::json!({ "content": [] }),
        is_error: false,
    });
    assert_eq!(
        app.ac().spinner.as_ref().unwrap().message(),
        "⏳ 6s (Esc to interrupt)"
    );
    // The panel row of a tracked child carries its compact label.
    app.handle_event(subagents_changed(vec![subagent_with_socket(
        "reviewer", "running", None, None,
    )]));
    app.handle_event(event(Some("reviewer"), waiting(4)));
    let rows = app.panel_rows_for_test();
    let reviewer = rows
        .iter()
        .find(|(label, _)| label == "reviewer")
        .expect("reviewer row");
    assert_eq!(reviewer.1.as_deref(), Some("4s"));
    let master = &rows[0];
    assert_eq!(
        master.1.as_deref(),
        Some("6s"),
        "master row uses the compact label"
    );
}

#[tokio::test(start_paused = true)]
async fn queued_panel_timers_advance_without_another_admission_event_and_clear_on_grant() {
    let mut app = test_app().await;
    app.handle_event(Event::AgentStart);
    app.handle_event(subagents_changed(vec![subagent_with_socket(
        "reviewer", "running", None, None,
    )]));
    app.handle_event(event(None, waiting(0)));
    app.handle_event(event(Some("reviewer"), waiting(0)));
    tokio::time::advance(std::time::Duration::from_secs(3)).await;
    let mut kitty_done = true;
    app.service_animation_tick(&mut kitty_done, tokio::time::Instant::now());
    assert_eq!(
        app.ac().master_session.footer.admission_compact(),
        Some("3s")
    );
    assert_eq!(
        app.ac()
            .roster
            .admission_labels
            .get("reviewer")
            .map(String::as_str),
        Some("3s")
    );
    for id in [None, Some("reviewer")] {
        app.handle_event(event(
            id,
            serde_json::json!({"waiting": 0, "admitted": 1, "revision": 2}),
        ));
    }
    tokio::time::advance(std::time::Duration::from_secs(2)).await;
    app.service_animation_tick(&mut kitty_done, tokio::time::Instant::now());
    assert_eq!(app.ac().master_session.footer.admission_compact(), None);
    assert!(app.ac().roster.admission_labels.is_empty());
}

#[tokio::test(start_paused = true)]
async fn timer_rebases_from_fresh_snapshots_preserves_unknown_and_saturates() {
    let mut app = test_app().await;
    let mut kitty_done = true;
    app.handle_event(event(None, waiting(12)));
    tokio::time::advance(std::time::Duration::from_secs(3)).await;
    app.service_animation_tick(&mut kitty_done, tokio::time::Instant::now());
    assert_eq!(
        app.ac().master_session.footer.admission_compact(),
        Some("15s")
    );
    app.handle_event(event(None, waiting(1)));
    tokio::time::advance(std::time::Duration::from_secs(2)).await;
    app.service_animation_tick(&mut kitty_done, tokio::time::Instant::now());
    assert_eq!(
        app.ac().master_session.footer.admission_compact(),
        Some("3s")
    );
    app.handle_event(event(None, serde_json::json!({"waiting": 1})));
    tokio::time::advance(std::time::Duration::from_secs(2)).await;
    app.service_animation_tick(&mut kitty_done, tokio::time::Instant::now());
    assert_eq!(
        app.ac().master_session.footer.admission_compact(),
        Some("waiting")
    );
    app.handle_event(event(None, waiting(u64::MAX)));
    tokio::time::advance(std::time::Duration::from_secs(2)).await;
    app.service_animation_tick(&mut kitty_done, tokio::time::Instant::now());
    assert_eq!(
        app.ac().master_session.footer.admission_compact(),
        Some(format!("{}s", u64::MAX).as_str())
    );
    app.handle_event(agent_end());
    tokio::time::advance(std::time::Duration::from_secs(2)).await;
    app.service_animation_tick(&mut kitty_done, tokio::time::Instant::now());
    assert_eq!(app.ac().master_session.footer.admission_compact(), None);
}

#[tokio::test(start_paused = true)]
async fn disconnected_feed_stops_forwarded_wait_timers() {
    let mut app = test_app().await;
    app.handle_event(subagents_changed(vec![subagent_with_socket(
        "reviewer", "running", None, None,
    )]));
    app.handle_event(event(Some("reviewer"), waiting(0)));
    app.mark_agent_disconnected_for_test();
    tokio::time::advance(std::time::Duration::from_secs(3)).await;
    app.service_animation_tick(&mut true, tokio::time::Instant::now());
    assert!(app.ac().roster.admission_labels.is_empty());
    assert!(app.ac().admission_children.is_empty());
}

#[tokio::test(start_paused = true)]
async fn rekeyed_child_keeps_wait_clock_without_another_transition() {
    let mut app = test_app().await;
    app.handle_event(subagents_changed(vec![subagent_with_socket(
        "reviewer", "running", None, None,
    )]));
    app.handle_event(event(Some("reviewer"), waiting(0)));
    let child = app.ac_mut().roster.tracked.remove("reviewer").unwrap();
    app.ac_mut().roster.tracked.insert("uuid".into(), child);
    app.rekey_agent_collections("reviewer", "uuid");
    tokio::time::advance(std::time::Duration::from_secs(3)).await;
    app.service_animation_tick(&mut true, tokio::time::Instant::now());
    assert_eq!(
        app.ac()
            .roster
            .admission_labels
            .get("uuid")
            .map(String::as_str),
        Some("3s")
    );
}

#[tokio::test(start_paused = true)]
async fn cooldowns_count_down_locally_and_expire_conservatively_for_master_and_child() {
    let mut app = test_app().await;
    app.handle_event(subagents_changed(vec![subagent_with_socket(
        "reviewer", "running", None, None,
    )]));
    app.handle_event(event(None, cooldown("anthropic", 3)));
    app.handle_event(event(Some("reviewer"), cooldown("anthropic", 3)));
    tokio::time::advance(std::time::Duration::from_secs(2)).await;
    app.service_animation_tick(&mut true, tokio::time::Instant::now());
    assert_eq!(
        app.ac().master_session.footer.admission(),
        Some("anthropic cooldown 1s")
    );
    assert_eq!(
        app.ac()
            .roster
            .admission_labels
            .get("reviewer")
            .map(String::as_str),
        Some("cooldown 1s")
    );
    tokio::time::advance(std::time::Duration::from_secs(1)).await;
    app.service_animation_tick(&mut true, tokio::time::Instant::now());
    assert_eq!(
        app.ac().master_session.footer.admission(),
        Some("anthropic cooldown elapsed")
    );
    assert_eq!(
        app.ac()
            .roster
            .admission_labels
            .get("reviewer")
            .map(String::as_str),
        Some("cooldown elapsed")
    );
}

#[tokio::test(start_paused = true)]
async fn expiry_guard_keeps_final_projection_armed_after_select_reentry() {
    let mut app = test_app().await;
    assert!(!app.needs_animation_tick(false));
    app.handle_event(event(None, cooldown("anthropic", 3)));
    tokio::time::advance(std::time::Duration::from_millis(2960)).await;
    app.service_animation_tick(&mut true, tokio::time::Instant::now());
    assert_eq!(
        app.ac().master_session.footer.admission(),
        Some("anthropic cooldown 1s")
    );
    // Another select arm wins after expiry, before the next 80ms tick.
    tokio::time::advance(std::time::Duration::from_millis(50)).await;
    assert!(
        app.needs_animation_tick(false),
        "select reentry must retain the final repaint"
    );
    assert!(app.service_animation_tick(&mut true, tokio::time::Instant::now()));
    assert_eq!(
        app.ac().master_session.footer.admission(),
        Some("anthropic cooldown elapsed")
    );
    assert!(
        !app.needs_animation_tick(false),
        "serviced expiry returns to idle"
    );
}
