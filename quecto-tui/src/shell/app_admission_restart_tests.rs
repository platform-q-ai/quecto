//! `/admission` and `/admission restart` (#2615), and the footer badge's
//! queue depth, through the headless harness: raw wire lines go through the
//! real `Event` deserializer and the footer through its real render.
use crate::shell::app::tui_harness::TuiHarness;

fn admission_line(admission: serde_json::Value) -> String {
    serde_json::json!({"type": "admission_state_changed", "admission": admission}).to_string()
}

fn response_line(success: bool, data: serde_json::Value, error: Option<&str>) -> String {
    let mut line = serde_json::json!({
        "type": "response", "id": "admission-restart",
        "command": "restart_admission", "success": success,
    });
    if success {
        line["data"] = data;
    } else {
        line["error"] = serde_json::json!(error);
    }
    line.to_string()
}

#[tokio::test]
async fn admission_restart_sends_the_restart_and_says_so() {
    let mut h = TuiHarness::new().await;
    h.submit("/admission restart");
    let commands = h.drain_commands().await;
    assert!(
        commands
            .iter()
            .any(|c| c.contains(r#""type":"restart_admission""#) && c.contains("admission-restart")),
        "{commands:?}"
    );
    assert!(
        h.notification_messages()
            .iter()
            .any(|m| m == "Restarting the admission broker…"),
        "{:?}",
        h.notification_messages()
    );
}

#[tokio::test]
async fn admission_with_anything_else_shows_the_usage_and_sends_nothing() {
    let mut h = TuiHarness::new().await;
    h.submit("/admission reboot");
    assert!(h.drain_commands().await.is_empty());
    assert_eq!(h.notification_messages(), ["Usage: /admission restart"]);
}

#[tokio::test]
async fn admission_alone_shows_the_brokers_health() {
    let mut h = TuiHarness::new().await;
    h.submit("/admission");
    assert_eq!(
        h.notification_messages(),
        ["This session has no admission broker"]
    );
    let mut h = TuiHarness::new().await;
    h.event_line(&admission_line(serde_json::json!({
        "waiting": 0, "revision": 1, "authorityStatus": "connected"
    })));
    h.submit("/admission");
    assert_eq!(
        h.notification_messages(),
        ["admission ✓ connected — Usage: /admission restart to restart the broker"]
    );
}

#[tokio::test]
async fn a_restart_response_says_how_the_broker_came_back() {
    let cases = [
        (
            serde_json::json!({"outcome": "restarted", "authorityStatus": "connected"}),
            "Admission broker restarted; this session reconnected",
        ),
        (
            serde_json::json!({"outcome": "service_restarted", "authorityStatus": "connected"}),
            "Admission broker service restarted; this session reconnected",
        ),
        (
            serde_json::json!({"outcome": "started", "authorityStatus": "reconnecting"}),
            "Admission broker started; this session is reconnecting",
        ),
    ];
    for (data, expected) in cases {
        let mut h = TuiHarness::new().await;
        h.event_line(&response_line(true, data, None));
        assert_eq!(h.notification_messages(), [expected]);
    }
    let mut h = TuiHarness::new().await;
    h.event_line(&response_line(
        false,
        serde_json::Value::Null,
        Some("an admission restart is already in progress"),
    ));
    assert_eq!(
        h.notification_messages(),
        ["Admission restart failed: an admission restart is already in progress"]
    );
}

#[tokio::test]
async fn the_badge_shows_health_and_this_sessions_queue_depth() {
    let mut h = TuiHarness::new().await;
    h.event_line(&admission_line(serde_json::json!({
        "waiting": 2, "longestWaitSeconds": 3, "revision": 1,
        "authorityStatus": "connected"
    })));
    let footer = h.master_footer_text();
    assert!(
        footer.contains("admission ✓ connected · 2 queued"),
        "{footer}"
    );
    // An activity-only event (no status) moves the depth, keeps the health.
    h.event_line(&admission_line(
        serde_json::json!({"waiting": 0, "revision": 2}),
    ));
    let footer = h.master_footer_text();
    assert!(footer.contains("admission ✓ connected"), "{footer}");
    assert!(!footer.contains("queued"), "{footer}");
    h.event_line(&admission_line(serde_json::json!({
        "waiting": 1, "revision": 3, "authorityStatus": "unavailable"
    })));
    let footer = h.master_footer_text();
    assert!(
        footer.contains("admission ✗ unavailable · 1 queued"),
        "{footer}"
    );
}

#[tokio::test]
async fn a_required_session_restart_is_offered_once_and_badged() {
    let mut h = TuiHarness::new().await;
    let changed = serde_json::json!({
        "waiting": 0, "revision": 1, "authorityStatus": "unavailable", "restartRequired": true
    });
    h.event_line(&admission_line(changed.clone()));
    let footer = h.master_footer_text();
    assert!(footer.contains("admission ✗ restart required"), "{footer}");
    assert_eq!(
        h.notification_messages(),
        [
            "The admission broker came back with a different policy; this session must restart to use it. /admission restart restarts the broker and this session."
        ]
    );
    // Still required: offered once, not on every event.
    let mut again = changed;
    again["revision"] = 2.into();
    h.event_line(&admission_line(again));
    assert_eq!(h.notification_messages().len(), 1);
    // An activity-only event keeps the badge.
    h.event_line(&admission_line(
        serde_json::json!({"waiting": 0, "revision": 3}),
    ));
    assert!(
        h.master_footer_text()
            .contains("admission ✗ restart required")
    );
}

fn session_restart_line() -> String {
    response_line(
        true,
        serde_json::json!({
            "outcome": "restarted", "authorityStatus": "unavailable",
            "sessionRestartRequired": true
        }),
        None,
    )
}

#[tokio::test]
async fn a_tui_that_owns_its_agent_restarts_the_session_for_a_changed_policy() {
    let mut h = TuiHarness::new().await;
    h.app_mut()
        .set_child_exit_watch(crate::shell::child_watch::ChildWatch::for_tests(Some(4242)));
    h.event_line(&session_restart_line());
    assert_eq!(
        h.notification_messages(),
        ["Admission broker restarted; restarting this session to use its policy…"]
    );
    assert!(
        h.should_exit(),
        "the ordinary exit ends the owned agent first"
    );
    assert!(h.app_mut().session_restart_requested());
}

#[tokio::test]
async fn a_tui_that_does_not_end_its_agent_says_what_to_restart() {
    for (owned, kill_on_exit) in [(false, true), (true, false)] {
        let mut h = TuiHarness::new().await;
        if owned {
            h.app_mut()
                .set_child_exit_watch(crate::shell::child_watch::ChildWatch::for_tests(Some(4242)));
        }
        h.app_mut().set_ordinary_exit_kill_owned(kill_on_exit);
        h.event_line(&session_restart_line());
        assert_eq!(
            h.notification_messages(),
            [
                "Admission broker restarted; its policy changed, and this TUI does not end its agent: restart that agent to use it"
            ]
        );
        assert!(!h.should_exit());
        assert!(!h.app_mut().session_restart_requested());
    }
}
