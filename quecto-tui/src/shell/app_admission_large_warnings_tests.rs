use super::*;
use crate::protocol::client::Event;
use crate::shell::app::app_events::app_events_test_support::test_app;

#[tokio::test]
async fn all_unbound_slots_beyond_sixty_four_are_reported_and_rearmed() {
    let mut app = test_app().await;
    let state = |count: usize| Event::Response {
        id: None,
        command: "get_state".into(),
        success: true,
        data: Some(
            serde_json::json!({"admissionWarnings": (0..count).map(|i| serde_json::json!({
            "slot": format!("provider-slot-{i}"), "code": "admission_binding_missing",
            "message": format!("provider-slot-{i} requests are not broker-gated")
        })).collect::<Vec<_>>()}),
        ),
        error: None,
    };
    app.handle_event(state(65));
    assert_eq!(app.shown_admission_warning_slots.len(), 65);
    assert!(
        app.shown_admission_warning_slots
            .contains("provider-slot-64")
    );
    let statuses = app
        .ac()
        .master_session
        .chat
        .entries()
        .iter()
        .filter_map(|entry| match entry {
            crate::components::chat::ChatEntry::Status { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        statuses.contains("provider-slot-64"),
        "last usable slot omitted: {statuses}"
    );
    let rendered = crate::components::ansi::strip_ansi(
        &app.ac_mut().master_session.chat.render(80).join("\n"),
    );
    assert!(rendered.contains("provider-slot-64"), "{rendered}");
    assert!(rendered.contains("admission.bindings"), "{rendered}");
    assert!(
        app.notifications
            .messages()
            .iter()
            .any(|m| m.contains("65 slots"))
    );
    app.handle_event(state(0));
    assert!(
        app.shown_admission_warning_slots.is_empty(),
        "complete bound snapshot must rearm"
    );
    app.handle_event(state(65));
    assert!(
        app.notifications
            .messages()
            .iter()
            .any(|m| m.contains("65 slots"))
    );
}
