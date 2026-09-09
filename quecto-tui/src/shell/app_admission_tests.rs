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
    assert_eq!(footer.admission(), Some("waiting for admission 12s"));
    assert_eq!(footer.admission_compact(), Some("waiting 12s"));
    assert_eq!(
        app.ac().spinner.as_ref().unwrap().message(),
        "Waiting for admission 12s (Esc to interrupt)"
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
        Some("waiting 4s")
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
    assert_eq!(
        app.ac().master_session.footer.admission(),
        Some("waiting for admission 2s")
    );
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
    assert_eq!(
        app.ac().master_session.footer.admission(),
        Some("waiting for admission 7s")
    );
    // An agent without an authority reports no view: the label is cleared.
    app.handle_event(response(None));
    assert_eq!(app.ac().master_session.footer.admission(), None);
    assert_eq!(capitalize(""), "");
    assert_eq!(capitalize("éa"), "Éa");
}
