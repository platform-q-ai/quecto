use super::*;
use crate::protocol::client::Event;
use crate::shell::app::app_events::app_events_test_support::test_app;

fn waiting(seconds: u64) -> serde_json::Value {
    serde_json::json!({
        "waiting": 1, "admitted": 0, "longestWaitSeconds": seconds, "revision": 1,
        "groups": [{"group": "anthropic"}]
    })
}

#[tokio::test]
async fn the_master_view_paints_the_footer_and_spinner_without_changing_state() {
    let mut app = test_app().await;
    app.handle_event(Event::AgentStart);
    assert!(app.ac().master_session.running);
    app.handle_event(Event::AdmissionStateChanged {
        agent_id: None,
        admission: waiting(12),
    });
    assert!(
        app.ac().master_session.running,
        "admission is not a lifecycle state"
    );
    assert_eq!(
        app.ac().master_session.footer.admission(),
        Some("waiting for admission 12s")
    );
    assert_eq!(
        app.ac().spinner.as_ref().unwrap().message(),
        "Waiting for admission 12s (Esc to interrupt)"
    );
    // Granted: the label leaves and the spinner returns to its plain message.
    app.handle_event(Event::AdmissionStateChanged {
        agent_id: Some(String::new()),
        admission: serde_json::json!({"waiting": 0, "admitted": 1, "revision": 2}),
    });
    assert_eq!(app.ac().master_session.footer.admission(), None);
    assert_eq!(
        app.ac().spinner.as_ref().unwrap().message(),
        WORKING_MESSAGE
    );
    // A cooldown-only view survives the end of the run; a waiting one does not.
    app.handle_event(Event::AdmissionStateChanged {
        agent_id: None,
        admission: serde_json::json!({
            "waiting": 0, "revision": 3,
            "groups": [{"group": "anthropic", "cooldown": {"state": "until", "remainingSeconds": 30}}]
        }),
    });
    assert_eq!(
        app.ac().spinner.as_ref().unwrap().message(),
        WORKING_MESSAGE
    );
    app.handle_event(Event::AgentEnd {
        messages: vec![],
        message_refs: vec![],
    });
    assert_eq!(
        app.ac().master_session.footer.admission(),
        Some("anthropic cooldown 30s")
    );
    app.handle_event(Event::AgentStart);
    app.handle_event(Event::AdmissionStateChanged {
        agent_id: None,
        admission: waiting(1),
    });
    app.handle_event(Event::AgentEnd {
        messages: vec![],
        message_refs: vec![],
    });
    assert_eq!(app.ac().master_session.footer.admission(), None);
    // Garbage is ignored.
    app.handle_event(Event::AdmissionStateChanged {
        agent_id: None,
        admission: serde_json::json!("nope"),
    });
    assert_eq!(app.ac().master_session.footer.admission(), None);
}

#[tokio::test]
async fn a_forwarded_child_view_labels_that_child_only() {
    let mut app = test_app().await;
    app.handle_event(Event::AdmissionStateChanged {
        agent_id: Some("reviewer".into()),
        admission: waiting(4),
    });
    assert_eq!(app.ac().master_session.footer.admission(), None);
    assert_eq!(
        app.ac()
            .roster
            .admission_labels
            .get("reviewer")
            .map(String::as_str),
        Some("waiting 4s")
    );
    app.handle_event(Event::AdmissionStateChanged {
        agent_id: Some("reviewer".into()),
        admission: serde_json::json!({"waiting": 0, "revision": 2}),
    });
    assert!(app.ac().roster.admission_labels.is_empty());
    // The connected agent's own id addresses the master.
    app.ac_mut().connected_agent_id = Some("me".into());
    app.handle_event(Event::AdmissionStateChanged {
        agent_id: Some("me".into()),
        admission: waiting(2),
    });
    assert_eq!(
        app.ac().master_session.footer.admission(),
        Some("waiting for admission 2s")
    );
}

#[tokio::test]
async fn get_state_applies_the_admission_view_like_the_event() {
    let mut app = test_app().await;
    app.handle_event(Event::Response {
        id: None,
        command: "get_state".into(),
        success: true,
        data: Some(serde_json::json!({
            "state": "thinking", "model": "m", "sessionKey": "cli:default",
            "progress": {"state": "waiting", "reason": "x"},
            "generation": 3,
            "admission": waiting(7)
        })),
        error: None,
    });
    assert_eq!(
        app.ac().master_session.footer.admission(),
        Some("waiting for admission 7s")
    );
    assert_eq!(capitalize(""), "");
    assert_eq!(capitalize("éa"), "Éa");
}
