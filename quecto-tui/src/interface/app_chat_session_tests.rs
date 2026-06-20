use super::tui_harness::TuiHarness;
use super::*;

async fn harness() -> TuiHarness {
    TuiHarness::new().await
}

#[tokio::test]
async fn resume_selector_renders_chat_metadata_and_uses_key_for_selection() {
    let mut h = harness().await;
    let data = serde_json::json!({
        "sessions": [{
            "key": "chat-123",
            "title": "Fix the auth bug",
            "name": "Fix the auth bug",
            "messageCount": 12,
            "updatedUnixSecs": 1781980920u64
        }]
    });
    let a = h.app_mut();

    a.open_resume_selector(&data);

    let selector = a.resume_selector.as_mut().unwrap();
    assert_eq!(selector.item_count(), 1);
    let rendered = selector.render_text(80);
    assert!(rendered.contains("Fix the auth bug"));
    assert!(rendered.contains("12 msgs"));
    assert!(
        rendered.contains("2026"),
        "date/time should be present: {rendered}"
    );

    a.handle_resume_selector_key(&Key::Enter);
    let sent = h.drain_commands().await;
    assert!(sent.iter().any(|cmd| {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(cmd) else {
            return false;
        };
        value.get("type").and_then(|v| v.as_str()) == Some("resume_session")
            && value.get("session").and_then(|v| v.as_str()) == Some("chat-123")
    }));
}

#[tokio::test]
async fn send_new_session_requests_fresh_chat_key() {
    let mut h = harness().await;
    let a = h.app_mut();

    a.send_new_session();

    let sent = h.drain_commands().await;
    assert!(sent.iter().any(|cmd| {
        serde_json::from_str::<serde_json::Value>(cmd)
            .ok()
            .and_then(|value| {
                value
                    .get("type")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .as_deref()
            == Some("new_session")
    }));
}
