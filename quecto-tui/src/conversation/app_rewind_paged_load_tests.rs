//! Focused rewind get_message paging regression tests, split out to keep
//! `app_rewind_response_tests.rs` within the line-count gate.

use super::tui_harness::TuiHarness;
use super::*;

async fn harness() -> TuiHarness {
    TuiHarness::new().await
}

fn respond(
    app: &mut App,
    id: Option<&str>,
    command: &str,
    success: bool,
    data: Option<serde_json::Value>,
    error: Option<&str>,
) {
    app.handle_event(Event::Response {
        id: id.map(str::to_string),
        command: command.to_string(),
        success,
        data,
        error: error.map(str::to_string),
    });
}

#[tokio::test]
async fn response_get_message_for_rewind_rejects_mismatched_message_id() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.ac_mut().rewind.pending_load_id = Some("load".into());
    a.ac_mut().rewind.pending_apply_message_id = Some("u1".into());
    let data = serde_json::json!({
        "id": "other",
        "role": "user",
        "content": "wrong prompt body",
        "contentLength": "wrong prompt body".len(),
        "hasMoreContent": false,
        "offset": 0
    });
    respond(a, Some("load"), "get_message", true, Some(data), None);
    assert!(a.ac().rewind.pending_load_id.is_none());
    assert!(a.ac().rewind.pending_apply_message_id.is_none());
    assert!(a.ac().rewind.pending_apply_id.is_none());
    assert!(a.ac().rewind.pending_apply_text.is_none());
    assert!(!a.notifications.is_empty());
}

#[tokio::test]
async fn response_get_message_for_rewind_pages_until_full_text_loaded() {
    let mut h = harness().await;
    {
        let a = h.app_mut();
        a.ac_mut().rewind.pending_load_id = Some("load-1".into());
        a.ac_mut().rewind.pending_apply_message_id = Some("u1".into());
        let data = serde_json::json!({
            "id": "u1",
            "role": "user",
            "content": "part one ",
            "contentLength": "part one part two".len(),
            "hasMoreContent": true,
            "nextOffset": "part one ".len(),
            "offset": 0
        });
        respond(a, Some("load-1"), "get_message", true, Some(data), None);
        assert!(a.ac().rewind.pending_load_id.is_some());
        assert_eq!(
            a.ac().rewind.pending_apply_message_id.as_deref(),
            Some("u1")
        );
        assert!(a.ac().rewind.pending_apply_id.is_none());
        assert_eq!(a.ac().rewind.pending_load_content, "part one ");
        assert_eq!(a.ac().rewind.pending_load_offset, "part one ".len());
        assert_eq!(
            a.ac().rewind.pending_load_content_len,
            Some("part one part two".len())
        );
    }

    let commands = h.drain_commands().await;
    let load = commands
        .iter()
        .find_map(|line| {
            let cmd = serde_json::from_str::<serde_json::Value>(line).ok()?;
            (cmd.get("type").and_then(|v| v.as_str()) == Some("get_message")).then_some(cmd)
        })
        .expect("next get_message page must be requested");
    assert_eq!(load.get("messageId").and_then(|v| v.as_str()), Some("u1"));
    assert_eq!(
        load.get("offset").and_then(|v| v.as_u64()),
        Some("part one ".len() as u64)
    );
    assert_eq!(
        load.get("limit").and_then(|v| v.as_u64()),
        Some(super::app_paged_history::GET_MESSAGE_PAGE_BYTES as u64)
    );

    let load_id = load.get("id").and_then(|v| v.as_str()).unwrap().to_string();
    let a = h.app_mut();
    let data = serde_json::json!({
        "id": "u1",
        "role": "user",
        "content": "part two",
        "contentLength": "part one part two".len(),
        "hasMoreContent": false,
        "offset": "part one ".len()
    });
    respond(a, Some(&load_id), "get_message", true, Some(data), None);
    assert!(a.ac().rewind.pending_load_id.is_none());
    assert!(a.ac().rewind.pending_apply_message_id.is_none());
    assert!(a.ac().rewind.pending_apply_id.is_some());
    assert_eq!(
        a.ac().rewind.pending_apply_text.as_deref(),
        Some("part one part two")
    );
}

#[tokio::test]
async fn response_get_message_for_rewind_rejects_changed_content_length_mid_load() {
    let mut h = harness().await;
    {
        let a = h.app_mut();
        a.ac_mut().rewind.pending_load_id = Some("load-1".into());
        a.ac_mut().rewind.pending_apply_message_id = Some("u1".into());
        let data = serde_json::json!({
            "id": "u1",
            "role": "user",
            "content": "part one ",
            "contentLength": "part one part two".len(),
            "hasMoreContent": true,
            "nextOffset": "part one ".len(),
            "offset": 0
        });
        respond(a, Some("load-1"), "get_message", true, Some(data), None);
    }

    let commands = h.drain_commands().await;
    let load_id = commands
        .iter()
        .find_map(|line| {
            let cmd = serde_json::from_str::<serde_json::Value>(line).ok()?;
            (cmd.get("type").and_then(|v| v.as_str()) == Some("get_message"))
                .then(|| cmd.get("id").and_then(|v| v.as_str()).unwrap().to_string())
        })
        .expect("next get_message page must be requested");

    let a = h.app_mut();
    let data = serde_json::json!({
        "id": "u1",
        "role": "user",
        "content": "part two",
        "contentLength": "part one part two changed".len(),
        "hasMoreContent": false,
        "offset": "part one ".len()
    });
    respond(a, Some(&load_id), "get_message", true, Some(data), None);
    assert!(a.ac().rewind.pending_load_id.is_none());
    assert!(a.ac().rewind.pending_apply_message_id.is_none());
    assert!(a.ac().rewind.pending_apply_id.is_none());
    assert!(a.ac().rewind.pending_apply_text.is_none());
    assert!(a.ac().rewind.pending_load_content_len.is_none());
    assert!(!a.notifications.is_empty());
}
