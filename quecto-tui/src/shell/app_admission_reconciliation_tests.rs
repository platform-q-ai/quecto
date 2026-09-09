use crate::protocol::client::Event;
use crate::shell::app::app_events::app_events_test_support::test_app;
use crate::shell::app::tui_harness::{subagent_with_socket, subagents_changed};

fn view(revision: u64, waiting: u64) -> serde_json::Value {
    serde_json::json!({"revision": revision, "waiting": waiting, "longestWaitSeconds": 4})
}
fn direct(admission: serde_json::Value) -> Event {
    Event::AdmissionStateChanged {
        agent_id: None,
        admission,
    }
}
fn snapshot(data: serde_json::Value) -> Event {
    Event::Response {
        id: None,
        command: "get_state".into(),
        success: true,
        data: Some(data),
        error: None,
    }
}

#[tokio::test]
async fn child_snapshot_restores_wait_and_direct_grant_rejects_stale_forwarding() {
    let mut app = test_app().await;
    app.handle_event(subagents_changed(vec![subagent_with_socket(
        "child", "running", None, None,
    )]));
    app.route_subagent_event(
        "child",
        snapshot(serde_json::json!({"admission": view(10, 1)})),
    );
    assert!(app.ac().roster.admission_labels.contains_key("child"));
    app.route_subagent_event("child", direct(view(11, 0)));
    assert!(app.ac().roster.admission_labels.is_empty());
    app.handle_event(Event::AdmissionStateChanged {
        agent_id: Some("child".into()),
        admission: view(10, 1),
    });
    app.route_subagent_event(
        "child",
        snapshot(serde_json::json!({"admission": view(10, 1)})),
    );
    assert!(
        app.ac().roster.admission_labels.is_empty(),
        "stale snapshot/event must not restore wait"
    );
    assert!(app.ac().master_session.footer.admission().is_none());
}

#[tokio::test(start_paused = true)]
async fn duplicate_child_events_preserve_elapsed_clock_and_terminal_clears_wait() {
    let mut app = test_app().await;
    app.handle_event(subagents_changed(vec![subagent_with_socket(
        "child", "running", None, None,
    )]));
    app.route_subagent_event("child", direct(view(1, 1)));
    let observed = app
        .ac()
        .admission_children
        .get("child")
        .expect("direct wait")
        .1;
    tokio::time::advance(std::time::Duration::from_secs(5)).await;
    app.route_subagent_event("child", direct(view(1, 1)));
    assert_eq!(app.ac().admission_children["child"].1, observed);
    app.route_subagent_event(
        "child",
        Event::AgentEnd {
            messages: vec![],
            message_refs: vec![],
        },
    );
    app.route_subagent_event("child", direct(view(1, 1)));
    app.tick_admission_labels();
    assert!(
        app.ac().roster.admission_labels.contains_key("child"),
        "same-revision authoritative state repairs an unversioned terminal clear"
    );
    app.route_subagent_event("child", direct(view(2, 1)));
    assert!(app.ac().roster.admission_labels.contains_key("child"));
    app.route_subagent_event("child", snapshot(serde_json::json!({})));
    app.route_subagent_event("child", direct(view(2, 1)));
    assert!(
        app.ac().roster.admission_labels.is_empty(),
        "absent snapshot clears and retains watermark"
    );
}

#[tokio::test]
async fn terminal_roster_clears_wait_and_rejects_delayed_events() {
    let mut app = test_app().await;
    app.handle_event(subagents_changed(vec![subagent_with_socket(
        "child", "running", None, None,
    )]));
    app.route_subagent_event("child", direct(view(1, 1)));
    app.handle_event(subagents_changed(vec![subagent_with_socket(
        "child", "dead", None, None,
    )]));
    assert!(app.ac().roster.admission_labels.is_empty());
    app.route_subagent_event("child", direct(view(2, 1)));
    app.tick_admission_labels();
    assert!(app.ac().roster.admission_labels.is_empty());
}

#[tokio::test]
async fn child_snapshot_failure_and_unknown_direct_targets_do_not_mutate_admission() {
    let mut app = test_app().await;
    app.handle_event(subagents_changed(vec![subagent_with_socket(
        "child", "running", None, None,
    )]));
    app.route_subagent_event(
        "child",
        Event::AdmissionStateChanged {
            agent_id: Some("child".into()),
            admission: view(1, 1),
        },
    );
    app.route_subagent_event(
        "child",
        Event::Response {
            id: None,
            command: "get_state".into(),
            success: false,
            data: Some(serde_json::json!({})),
            error: Some("failed".into()),
        },
    );
    app.route_subagent_event("child", direct(serde_json::json!("malformed")));
    assert!(app.ac().roster.admission_labels.contains_key("child"));
    app.route_subagent_event(
        "child",
        Event::AdmissionStateChanged {
            agent_id: Some("unknown".into()),
            admission: view(1, 1),
        },
    );
    app.route_subagent_event("unknown", direct(view(1, 1)));
    assert_eq!(app.ac().admission_children.len(), 1);
    assert!(app.ac().master_session.footer.admission().is_none());
}

#[tokio::test(start_paused = true)]
async fn child_turn_end_clears_wait_but_preserves_cooldown() {
    let mut app = test_app().await;
    app.handle_event(subagents_changed(vec![subagent_with_socket(
        "child", "running", None, None,
    )]));
    app.route_subagent_event(
        "child",
        direct(serde_json::json!({
            "revision": 4, "waiting": 1, "longestWaitSeconds": 9,
            "groups": [{"group": "test", "cooldown": {"state": "until", "remainingSeconds": 30}}]
        })),
    );
    tokio::time::advance(std::time::Duration::from_secs(10)).await;
    app.route_subagent_event(
        "child",
        Event::TurnEnd {
            message: serde_json::json!({}),
        },
    );
    assert_eq!(
        app.ac().admission_children["child"].0.waiting,
        1,
        "unversioned terminal must not mutate the authoritative snapshot"
    );
    assert_eq!(
        app.ac()
            .roster
            .admission_labels
            .get("child")
            .map(String::as_str),
        Some("cooldown 20s")
    );
    for (seconds, expected) in [(5, "cooldown 15s"), (15, "cooldown elapsed")] {
        tokio::time::advance(std::time::Duration::from_secs(seconds)).await;
        assert!(app.service_animation_tick(&mut true, tokio::time::Instant::now()));
        assert_eq!(app.ac().roster.admission_labels["child"], expected);
        assert_eq!(app.ac().admission_children["child"].0.waiting, 1);
    }
    app.route_subagent_event("child", direct(view(4, 1)));
    assert_eq!(
        app.ac().admission_children["child"].0.waiting,
        1,
        "same-revision authoritative state repairs an unversioned terminal clear"
    );
}

#[tokio::test]
async fn metadata_only_unchanged_snapshot_preserves_wait_and_same_revision_repair() {
    let mut app = test_app().await;
    app.handle_event(subagents_changed(vec![subagent_with_socket(
        "child", "running", None, None,
    )]));
    app.route_subagent_event("child", direct(view(10, 1)));
    app.route_subagent_event(
        "child",
        snapshot(serde_json::json!({"unchanged": true, "generation": 42})),
    );
    assert!(app.ac().roster.admission_labels.contains_key("child"));
    app.route_subagent_event(
        "child",
        snapshot(serde_json::json!({"admission": view(10, 1)})),
    );
    assert!(app.ac().roster.admission_labels.contains_key("child"));
}

#[tokio::test]
async fn delayed_direct_terminal_allows_same_revision_authoritative_repair() {
    let mut app = test_app().await;
    app.handle_event(subagents_changed(vec![subagent_with_socket(
        "child", "running", None, None,
    )]));
    app.handle_event(Event::AdmissionStateChanged {
        agent_id: Some("child".into()),
        admission: view(12, 1),
    });
    app.route_subagent_event(
        "child",
        Event::TurnEnd {
            message: serde_json::json!({}),
        },
    );
    assert!(app.ac().roster.admission_labels.is_empty());
    app.route_subagent_event("child", direct(view(12, 1)));
    assert!(
        app.ac().roster.admission_labels.contains_key("child"),
        "same-revision direct state must repair an unversioned terminal clear"
    );
}

#[tokio::test]
async fn rekey_waiting_alias_cannot_restore_label_over_granted_uuid() {
    let mut app = test_app().await;
    app.handle_event(subagents_changed(vec![
        subagent_with_socket("alias", "running", None, None),
        subagent_with_socket("uuid", "running", None, None),
    ]));
    app.route_subagent_event("alias", direct(view(10, 1)));
    app.route_subagent_event("uuid", direct(view(11, 0)));
    app.rekey_agent_collections("alias", "uuid");
    assert!(
        app.ac().roster.admission_labels.is_empty(),
        "winning grant and label must move together"
    );
    app.tick_admission_labels();
    assert!(app.ac().roster.admission_labels.is_empty());
    assert_eq!(app.ac().admission_children["uuid"].0.revision, 11);
}
