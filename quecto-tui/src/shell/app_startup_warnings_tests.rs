//! #2435 review round 1 L6: the startup model's warnings reach the user
//! once, whatever the number of `get_state` refreshes.

use crate::components::chat::ChatEntry;
use crate::protocol::client::Event;
use crate::shell::app::app_events::app_events_test_support::test_app;

const WARNING: &str = "agent: warning: model `openai-oauth/gpt-5.5` was retired from the built-in models of `openai-oauth` (#2435); it is sent as-is with no known limits.";

fn state(warnings: &[&str]) -> Event {
    Event::Response {
        id: None,
        command: "get_state".into(),
        success: true,
        data: Some(
            serde_json::json!({"model": "openai-oauth/gpt-5.5", "startupWarnings": warnings}),
        ),
        error: None,
    }
}

#[tokio::test]
async fn a_startup_warning_is_shown_once_across_state_refreshes() {
    let mut app = test_app().await;
    app.handle_event(state(&[WARNING]));
    app.handle_event(state(&[WARNING]));
    app.handle_event(state(&[]));
    let shown: Vec<&str> = app
        .ac()
        .master_session
        .chat
        .entries()
        .iter()
        .filter_map(|entry| match entry {
            ChatEntry::Status { text } => Some(text.as_str()),
            _ => None,
        })
        .filter(|text| text.contains("was retired"))
        .collect();
    assert_eq!(shown, [WARNING], "shown once, in the transcript");
    assert!(app.shown_startup_warnings.contains(WARNING));
}
