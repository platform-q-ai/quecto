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
