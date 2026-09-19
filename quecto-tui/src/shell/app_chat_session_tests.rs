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
            "resumeEligible": true,
            "updatedUnixSecs": 1781980920u64
        }]
    });
    let a = h.app_mut();

    a.open_resume_selector(&data);

    let selector = a.ac_mut().sessions.resume_selector.as_mut().unwrap();
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
        let Ok(value) = serde_json::from_str::<serde_json::Value>(cmd) else {
            return false;
        };
        value.get("type").and_then(|v| v.as_str()) == Some("new_session")
            && value.get("id").is_none()
    }));
}

#[test]
fn format_utc_minutes_formats_known_timestamps() {
    use super::app_methods::format_utc_minutes;
    // 1_700_000_000 = 2023-11-14 22:13:20 UTC (the UTC fallback path).
    assert_eq!(format_utc_minutes(1_700_000_000), "2023-11-14 22:13");
    assert_eq!(format_utc_minutes(0), "1970-01-01 00:00");
}

#[tokio::test]
async fn scoped_discovery_discards_old_answers_and_cancel_never_restores() {
    use crate::protocol::session_payloads::SessionListScope;
    let mut h = harness().await;
    let a = h.app_mut();
    a.send_list_sessions();
    let old = a.ac().sessions.pending_list_id.clone().unwrap();
    a.request_session_scope(SessionListScope::Global);
    let current = a.ac().sessions.pending_list_id.clone().unwrap();
    let data = serde_json::json!({"sessions":[{"key":"foreign","title":"Foreign","executionPath":"/elsewhere","resumeEligible":false}]});
    a.handle_session_list_response(Some(&old), Some(data.clone()));
    assert_eq!(
        a.ac()
            .sessions
            .resume_selector
            .as_ref()
            .unwrap()
            .item_count(),
        0
    );
    a.handle_session_list_response(Some(&current), Some(data));
    assert!(a.ac().sessions.resume_selector.is_some());
    // #2011: an ineligible row is not decided here — the harness is asked and
    // answers with the typed decision; Escape there restores and sends nothing.
    // What replaced the client-side block: from the send until the harness
    // answers `resumed`, nothing local changes — identity, chat.
    a.ac_mut().session_key = Some("cli:showing".into());
    a.ac_mut()
        .master_session
        .chat
        .add_entry(crate::components::chat::ChatEntry::User {
            text: "the conversation on screen".into(),
        });
    let snapshot = |a: &mut App| {
        let chat = a.ac_mut().master_session.chat.render(120).join("\n");
        (a.ac().session_key.clone(), chat)
    };
    let before = snapshot(a);
    assert!(
        before.1.contains("the conversation on screen"),
        "{before:?}"
    );
    a.handle_resume_selector_key(&Key::Enter);
    let asked = a.ac().pending_session_resume_id.clone().expect("asked");
    assert!(a.ac().sessions.resume_selector.is_none());
    assert_eq!(snapshot(a), before, "sending changes nothing");
    let decision = serde_json::json!({
        "outcome": "decision", "code": "decision_required", "session": "foreign",
        "sessionKey": "foreign", "kind": "cross_folder", "homeVersion": "h1-0123456789abcdef",
        "executionPath": "/elsewhere", "detail": null,
        "actions": [{"action": "cancel", "available": true, "reason": null}],
    });
    a.handle_response(
        Some(asked),
        "resume_session".into(),
        false,
        Some(decision),
        Some("session resume unavailable".into()),
    );
    assert!(a.ac().sessions.resume_decision.is_some());
    assert!(a.ac().pending_session_resume_id.is_none());
    assert_eq!(snapshot(a), before, "a decision changes nothing");
    a.handle_resume_selector_key(&Key::Escape);
    assert!(a.ac().sessions.resume_decision.is_none());
    assert!(a.ac().pending_session_resume_id.is_none());
    assert_eq!(snapshot(a), before, "Escape changes nothing");
    a.handle_session_list_response(Some(&current), Some(serde_json::json!({"sessions":[]})));
    assert!(a.ac().sessions.resume_selector.is_none());
}

#[tokio::test]
async fn pending_discovery_escape_cancels_before_first_response() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.send_list_sessions();
    let id = a.ac().sessions.pending_list_id.clone().unwrap();
    a.handle_resume_selector_key(&Key::Escape);
    a.handle_session_list_response(
        Some(&id),
        Some(serde_json::json!({"sessions":[{"key":"x","title":"X","resumeEligible":true}]})),
    );
    assert!(a.ac().sessions.resume_selector.is_none());
    assert!(a.ac().pending_session_resume_id.is_none());
}

#[tokio::test]
async fn discovery_mouse_global_uses_full_frame_coordinates_with_panel() {
    let mut h = TuiHarness::sized(180, 50).await;
    h.app_mut().send_list_sessions();
    let frame = h.full_frame();
    let (y, line) = frame
        .lines()
        .enumerate()
        .find(|(_, line)| line.contains("[Local Folder]   All Folders"))
        .unwrap();
    let x = line[..line.find("All Folders").unwrap()].chars().count();
    h.press(Key::MousePress(x as u16, y as u16));
    assert_eq!(
        h.app_mut().ac().sessions.scope,
        crate::protocol::session_payloads::SessionListScope::Global
    );
}

/// R2-L5: an error answer to this tab's own list request closes the picker
/// `request_session_scope` opened for it; a foreign or stale error leaves the
/// live request and its picker alone.
#[tokio::test]
async fn list_sessions_error_closes_the_picker_it_opened() {
    use crate::protocol::session_payloads::SessionListScope;
    let mut h = harness().await;
    let a = h.app_mut();
    a.send_list_sessions();
    let stale = a.ac().sessions.pending_list_id.clone().unwrap();
    a.request_session_scope(SessionListScope::Global);
    let current = a.ac().sessions.pending_list_id.clone().unwrap();
    assert!(a.ac().sessions.resume_selector.is_some());
    a.handle_response(
        Some(stale),
        "list_sessions".into(),
        false,
        None,
        Some("stale".into()),
    );
    assert!(a.ac().sessions.resume_selector.is_some());
    assert_eq!(
        a.ac().sessions.pending_list_id.as_deref(),
        Some(current.as_str())
    );
    a.handle_response(
        Some(current),
        "list_sessions".into(),
        false,
        None,
        Some("discovery unavailable".into()),
    );
    assert!(a.ac().sessions.resume_selector.is_none());
    assert!(a.ac().sessions.pending_list_id.is_none());
    let rendered = a.notifications.render(200).join("\n");
    assert!(
        rendered.contains("Could not list sessions: discovery unavailable"),
        "{rendered}"
    );
}

/// #2018: a corrupt record's diagnostic toasts once per process, not on every
/// listing in every scope, and several new diagnostics collapse to one line.
#[tokio::test]
async fn discovery_diagnostics_toast_once_per_process_and_summarise_batches() {
    use crate::protocol::session_payloads::SessionListScope;
    let mut h = harness().await;
    let a = h.app_mut();
    let diag =
        "cli_slippery-keith.json: session record unavailable: expected value at line 79 column 6";
    for _ in 0..2 {
        for scope in [SessionListScope::Local, SessionListScope::Global] {
            a.request_session_scope(scope);
            let id = a.ac().sessions.pending_list_id.clone().unwrap();
            a.handle_session_list_response(
                Some(&id),
                Some(serde_json::json!({"sessions":[],"diagnostics":[diag]})),
            );
        }
    }
    let toasts: Vec<String> = a
        .notifications
        .messages()
        .into_iter()
        .filter(|m| m.contains("session record unavailable"))
        .collect();
    assert_eq!(toasts, vec![diag.to_string()]);
    a.request_session_scope(SessionListScope::Local);
    let id = a.ac().sessions.pending_list_id.clone().unwrap();
    a.handle_session_list_response(
        Some(&id),
        Some(serde_json::json!({"sessions":[],"diagnostics":[diag, "a.json: session record unavailable: EOF", "b.json: home needs repair: gone"]})),
    );
    let messages = a.notifications.messages();
    assert!(
        messages.iter().any(|m| m
            == "2 session discovery problems: a.json, b.json; first: a.json: session record unavailable: EOF"),
        "{messages:?}"
    );
}
