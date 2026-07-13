//! #1060 lifecycle regressions: transcript-changing responses invalidate pending
//! ref recovery so late lookups from an old transcript cannot splice into the new one.

use super::tui_harness::TuiHarness;
use super::*;

fn chat_text(app: &mut App) -> String {
    app.master_session
        .chat
        .render(120)
        .iter()
        .map(|l| super::app_methods::strip_ansi(l))
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_get_message_cmd(line: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(line)
        .ok()
        .and_then(|v| {
            v.get("type")
                .and_then(|t| t.as_str())
                .map(|s| s == "get_message")
        })
        .unwrap_or(false)
}

fn get_message_ids(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter(|l| is_get_message_cmd(l))
        .filter_map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).ok()?;
            v.get("id").and_then(|i| i.as_str()).map(str::to_string)
        })
        .collect()
}

async fn pending_recovery_id(h: &mut TuiHarness, ref_id: &str) -> String {
    {
        let a = h.app_mut();
        a.handle_event(Event::AgentStart);
        a.handle_event(Event::Token {
            token: "partial".into(),
        });
        a.handle_event(Event::TurnEnd {
            message: serde_json::json!({
                "role": "assistant", "content": "", "messageRefs": [ref_id], "contentLength": 64
            }),
        });
    }
    get_message_ids(&h.drain_commands().await)
        .into_iter()
        .next()
        .expect("recovery request id")
}

#[tokio::test]
async fn clear_history_drops_pending_recovery_so_late_response_is_ignored() {
    let mut h = TuiHarness::new().await;
    let ref_id = "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee";
    let recovery_id = pending_recovery_id(&mut h, ref_id).await;

    let a = h.app_mut();
    a.handle_event(Event::Response {
        id: Some("clear".into()),
        command: "clear_history".into(),
        success: true,
        data: None,
        error: None,
    });
    a.handle_event(Event::Response {
        id: Some(recovery_id),
        command: "get_message".into(),
        success: true,
        data: Some(serde_json::json!({
            "id": ref_id, "role": "assistant", "content": "STALE_RECOVERY_AFTER_CLEAR"
        })),
        error: None,
    });
    let frame = chat_text(a);
    assert!(
        !frame.contains("STALE_RECOVERY_AFTER_CLEAR"),
        "late recovery after clear_history must be ignored:\n{frame}"
    );
}

#[tokio::test]
async fn resume_session_drops_pending_recovery_so_late_response_is_ignored() {
    let mut h = TuiHarness::new().await;
    let ref_id = "eeeeeeee-eeee-eeee-eeee-ffffffffffff";
    let recovery_id = pending_recovery_id(&mut h, ref_id).await;

    let a = h.app_mut();
    a.handle_event(Event::Response {
        id: Some("resume".into()),
        command: "resume_session".into(),
        success: true,
        data: Some(serde_json::json!({"session":"other"})),
        error: None,
    });
    a.handle_event(Event::Response {
        id: Some(recovery_id),
        command: "get_message".into(),
        success: true,
        data: Some(serde_json::json!({
            "id": ref_id, "role": "assistant", "content": "STALE_RECOVERY_AFTER_RESUME"
        })),
        error: None,
    });
    let frame = chat_text(a);
    assert!(
        !frame.contains("STALE_RECOVERY_AFTER_RESUME"),
        "late recovery after resume_session must be ignored:\n{frame}"
    );
}

#[tokio::test]
async fn rewind_to_drops_pending_recovery_so_late_response_is_ignored() {
    let mut h = TuiHarness::new().await;
    let ref_id = "eeeeeeee-eeee-eeee-eeee-000000000000";
    let recovery_id = pending_recovery_id(&mut h, ref_id).await;

    let a = h.app_mut();
    a.rewind.pending_apply_id = Some("rw-apply".into());
    a.handle_event(Event::Response {
        id: Some("rw-apply".into()),
        command: "rewind_to".into(),
        success: true,
        data: None,
        error: None,
    });
    a.handle_event(Event::Response {
        id: Some(recovery_id),
        command: "get_message".into(),
        success: true,
        data: Some(serde_json::json!({
            "id": ref_id, "role": "assistant", "content": "STALE_RECOVERY_AFTER_REWIND"
        })),
        error: None,
    });
    let frame = chat_text(a);
    assert!(
        !frame.contains("STALE_RECOVERY_AFTER_REWIND"),
        "late recovery after rewind_to must be ignored:\n{frame}"
    );
}

#[tokio::test]
async fn new_session_drops_pending_recovery_so_late_response_is_ignored() {
    let mut h = TuiHarness::new().await;
    let ref_id = "eeeeeeee-eeee-eeee-eeee-111111111111";
    let recovery_id = pending_recovery_id(&mut h, ref_id).await;

    // The REAL /clear and /new user path lands in reset_session (the wired
    // clear_history response arm only fires from a test-only sender).
    h.app_mut().reset_session("New session started");

    h.app_mut().handle_event(Event::Response {
        id: Some(recovery_id),
        command: "get_message".into(),
        success: true,
        data: Some(serde_json::json!({
            "id": ref_id, "role": "assistant", "content": "STALE_RECOVERY_AFTER_NEW"
        })),
        error: None,
    });
    let frame = chat_text(h.app_mut());
    assert!(
        !frame.contains("STALE_RECOVERY_AFTER_NEW"),
        "late recovery after /new (reset_session) must be ignored:\n{frame}"
    );
}
