//! The `/resume` refusal flow in the shell (#2045): a home refusal owned by
//! this connection opens the plain notice and settles the latch; a peer's or
//! an id-less one opens nothing; the notice sends nothing, whatever key closes
//! it; any other refusal is a toast; a picker selection echoes the version
//! its row was listed at and a typed key borrows none.
use super::super::app_paged_history_tests::harness;
use super::super::*;
use serde_json::json;

fn elsewhere() -> serde_json::Value {
    json!({
        "outcome": "refused", "code": "belongs_elsewhere", "kind": "cross_folder",
        "session": "cli:foreign", "sessionKey": "cli:foreign",
        "executionPath": "/work/other", "detail": null,
        "command": "cd '/work/other' && quecto-tui", "resume": "/resume cli:foreign",
    })
}

/// Send `/resume cli:foreign` and answer it with `data` as a refusal.
async fn asked_and_answered(
    h: &mut super::super::tui_harness::TuiHarness,
    data: serde_json::Value,
) {
    h.app_mut().ac_mut().agent_connected = true;
    h.app_mut().send_resume_session("cli:foreign");
    let id = h.app_mut().ac().pending_session_resume_id.clone().unwrap();
    let _ = h.drain_commands().await;
    h.app_mut().handle_response(
        Some(id),
        "resume_session".into(),
        false,
        Some(data),
        Some("session resume unavailable: …".into()),
    );
}

fn resume_commands(commands: &[String]) -> Vec<serde_json::Value> {
    commands
        .iter()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|command| command["type"] == "resume_session")
        .collect()
}

#[tokio::test]
async fn an_owned_home_refusal_opens_the_notice_and_settles_the_latch() {
    let mut h = harness().await;
    asked_and_answered(&mut h, elsewhere()).await;
    let app = h.app_mut();
    assert!(app.ac().sessions.resume_decision.is_some());
    assert!(app.ac().sessions.has_modal());
    assert!(app.ac().pending_session_resume_id.is_none());
    let frame = h.full_frame();
    assert!(
        frame.contains("This session belongs to another folder"),
        "{frame}"
    );
    assert!(frame.contains("cd '/work/other' && quecto-tui"), "{frame}");
    assert!(frame.contains("/resume cli:foreign"), "{frame}");
    assert!(
        !frame.contains("quecto-tui -s"),
        "quecto-tui has no -s: {frame}"
    );
}

#[tokio::test]
async fn a_peers_home_refusal_opens_nothing_here() {
    let mut h = harness().await;
    h.app_mut().handle_response(
        Some("other-tab:resume-1".into()),
        "resume_session".into(),
        false,
        Some(elsewhere()),
        Some("session resume unavailable".into()),
    );
    assert!(h.app_mut().ac().sessions.resume_decision.is_none());
    assert!(!h.app_mut().ac().sessions.has_modal());
}

/// Review R2-T9 (#2011): a refusal with NO id is nobody's in particular — even
/// with this connection's resume in flight it opens nothing and the request
/// stays in flight for its own, correlated answer.
#[tokio::test]
async fn an_idless_home_refusal_opens_nothing_and_settles_nothing() {
    let mut h = harness().await;
    h.app_mut().ac_mut().agent_connected = true;
    h.app_mut().send_resume_session("cli:foreign");
    let id = h.app_mut().ac().pending_session_resume_id.clone().unwrap();
    let _ = h.drain_commands().await;
    h.app_mut().handle_response(
        None,
        "resume_session".into(),
        false,
        Some(elsewhere()),
        Some("session resume unavailable".into()),
    );
    assert!(h.app_mut().ac().sessions.resume_decision.is_none());
    assert!(!h.app_mut().ac().sessions.has_modal());
    assert_eq!(
        h.app_mut().ac().pending_session_resume_id.as_deref(),
        Some(id.as_str())
    );
}

#[tokio::test]
async fn every_key_that_closes_the_notice_sends_nothing() {
    for key in [Key::Escape, Key::Enter, Key::Ctrl('c')] {
        let mut h = harness().await;
        asked_and_answered(&mut h, elsewhere()).await;
        let key_before = h.app_mut().ac().session_key.clone();
        h.app_mut().handle_key(key.clone());
        assert!(
            h.app_mut().ac().sessions.resume_decision.is_none(),
            "{key:?}"
        );
        let sent = h.drain_commands().await;
        assert!(sent.is_empty(), "{key:?} sent {sent:?}");
        assert_eq!(h.app_mut().ac().session_key, key_before);
    }
}

#[tokio::test]
async fn typing_with_the_notice_open_reaches_neither_the_editor_nor_the_harness() {
    let mut h = harness().await;
    asked_and_answered(&mut h, elsewhere()).await;
    for ch in "y/new".chars() {
        h.app_mut().handle_key(Key::Char(ch));
    }
    assert!(h.app_mut().ac().sessions.resume_decision.is_some());
    assert!(h.drain_commands().await.is_empty());
    h.app_mut().handle_key(Key::Escape);
    assert!(
        !h.full_frame().contains("y/new"),
        "typed text leaked into the editor"
    );
}

#[tokio::test]
async fn a_refusal_that_is_not_about_the_folder_is_a_toast_not_a_notice() {
    let mut h = harness().await;
    asked_and_answered(
        &mut h,
        json!({"outcome": "refused", "code": "claim_refused"}),
    )
    .await;
    assert!(h.app_mut().ac().sessions.resume_decision.is_none());
    let frame = h.full_frame();
    assert!(frame.contains("Resume failed"), "{frame}");
}

/// An older harness still answers with its decision payload (`actions` and
/// all): the same notice, from kind and folder, with nothing to run.
#[tokio::test]
async fn an_older_harnesss_decision_shaped_refusal_still_opens_the_notice() {
    let mut h = harness().await;
    asked_and_answered(
        &mut h,
        json!({
            "outcome": "decision", "code": "decision_required", "kind": "cross_folder",
            "session": "cli:foreign", "sessionKey": "cli:foreign",
            "homeVersion": "h1-0123456789abcdef",
            "executionPath": "/work/gone", "detail": "No such file or directory",
            "actions": [{"action": "cancel", "available": true, "reason": null}],
        }),
    )
    .await;
    let frame = h.full_frame();
    assert!(
        frame.contains("This session belongs to another folder"),
        "{frame}"
    );
    assert!(frame.contains("/work/gone"), "{frame}");
    assert!(
        frame.contains("Open quecto in that folder and resume it there."),
        "{frame}"
    );
    assert!(
        !frame.contains("cd '"),
        "an older harness sent no command: {frame}"
    );
}

#[tokio::test]
async fn a_picker_selection_echoes_the_listed_version_even_for_an_ineligible_row() {
    let mut h = harness().await;
    h.app_mut().ac_mut().agent_connected = true;
    let data = json!({"sessions": [{
        "name": "elsewhere", "key": "chat-1-else", "messageCount": 3,
        "resumeEligible": false, "homeVersion": "h1-00000000000000aa",
    }]});
    h.app_mut().open_resume_selector(&data);
    h.app_mut().handle_key(Key::Enter);
    assert!(h.app_mut().ac().sessions.resume_selector.is_none());
    let sent = resume_commands(&h.drain_commands().await);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0]["session"], "chat-1-else");
    assert_eq!(sent[0]["expectedHomeVersion"], "h1-00000000000000aa");
    assert!(sent[0].get("action").is_none(), "{sent:?}");
}

#[tokio::test]
async fn a_typed_key_never_borrows_another_rows_version() {
    let mut h = harness().await;
    h.app_mut().ac_mut().agent_connected = true;
    h.app_mut().ac_mut().sessions.selected_home_version =
        Some(("chat-1-else".into(), "h1-00000000000000aa".into()));
    h.app_mut().send_resume_session("cli:typed");
    let sent = resume_commands(&h.drain_commands().await);
    assert_eq!(sent[0]["session"], "cli:typed");
    assert!(sent[0].get("expectedHomeVersion").is_none(), "{sent:?}");
    assert!(h.app_mut().ac().sessions.selected_home_version.is_none());
}

#[tokio::test]
async fn closing_the_session_overlays_closes_the_notice() {
    let mut h = harness().await;
    asked_and_answered(&mut h, elsewhere()).await;
    h.app_mut().close_session_switch_overlays();
    assert!(h.app_mut().ac().sessions.resume_decision.is_none());
}

/// The notice names the session the way the picker did: its title.
#[tokio::test]
async fn the_notice_repeats_the_title_the_picker_showed() {
    let mut h = harness().await;
    h.app_mut().ac_mut().agent_connected = true;
    let data = json!({"sessions": [{
        "key": "cli:foreign", "title": "Fix the flaky renderer", "messageCount": 3,
        "resumeEligible": false, "homeVersion": "h1-00000000000000aa",
    }]});
    h.app_mut().open_resume_selector(&data);
    h.app_mut().handle_key(Key::Enter);
    let id = h.app_mut().ac().pending_session_resume_id.clone().unwrap();
    let _ = h.drain_commands().await;
    h.app_mut().handle_response(
        Some(id),
        "resume_session".into(),
        false,
        Some(elsewhere()),
        Some("session resume unavailable".into()),
    );
    let frame = h.full_frame();
    assert!(frame.contains("Fix the flaky renderer"), "{frame}");
}

#[path = "app_resume_answer_tests.rs"]
mod answers;
