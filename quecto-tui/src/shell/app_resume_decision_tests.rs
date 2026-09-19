//! The `/resume` decision flow in the shell (#2011): a decision answer owned
//! by this tab opens the dialog; Cancel and Escape send nothing; an
//! unavailable action is explained and never sent; a picker selection echoes
//! the version its row was listed at.
use super::super::app_paged_history_tests::harness;
use super::super::*;
use serde_json::json;

fn decision(open_available: bool) -> serde_json::Value {
    json!({
        "outcome": "decision", "code": "decision_required",
        "session": "cli:foreign", "sessionKey": "cli:foreign",
        "kind": "cross_folder", "homeVersion": "h1-0123456789abcdef",
        "executionPath": "/work/other", "detail": null,
        "actions": [
            {"action": "open_original", "available": open_available, "reason": "not yet (#2012)"},
            {"action": "fork_current", "available": false, "reason": "not yet (#2013)"},
            {"action": "cancel", "available": true, "reason": null},
        ],
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
async fn an_owned_decision_opens_the_dialog_and_settles_the_latches() {
    let mut h = harness().await;
    asked_and_answered(&mut h, decision(false)).await;
    let app = h.app_mut();
    assert!(app.ac().sessions.resume_decision.is_some());
    assert!(app.ac().sessions.has_modal());
    assert!(app.ac().pending_session_resume_id.is_none());
    let frame = h.full_frame();
    assert!(frame.contains("belongs to another folder"), "{frame}");
    assert!(frame.contains("Open original folder"), "{frame}");
}

#[tokio::test]
async fn a_peers_decision_opens_nothing_here() {
    let mut h = harness().await;
    h.app_mut().handle_response(
        Some("other-tab:resume-1".into()),
        "resume_session".into(),
        false,
        Some(decision(false)),
        Some("session resume unavailable".into()),
    );
    assert!(h.app_mut().ac().sessions.resume_decision.is_none());
    assert!(!h.app_mut().ac().sessions.has_modal());
}

/// Review R2-T9: a decision with NO id is nobody's in particular — even with
/// this tab's resume in flight it opens nothing and the request stays in
/// flight for its own, correlated answer.
#[tokio::test]
async fn an_idless_decision_opens_nothing_and_settles_nothing() {
    let mut h = harness().await;
    h.app_mut().ac_mut().agent_connected = true;
    h.app_mut().send_resume_session("cli:foreign");
    let id = h.app_mut().ac().pending_session_resume_id.clone().unwrap();
    let _ = h.drain_commands().await;
    h.app_mut().handle_response(
        None,
        "resume_session".into(),
        false,
        Some(decision(false)),
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
async fn escape_and_cancel_close_the_dialog_and_send_nothing() {
    for keys in [vec![Key::Escape], vec![Key::Down, Key::Down, Key::Enter]] {
        let mut h = harness().await;
        asked_and_answered(&mut h, decision(false)).await;
        for key in keys {
            h.app_mut().handle_key(key);
        }
        assert!(h.app_mut().ac().sessions.resume_decision.is_none());
        let commands = h.drain_commands().await;
        assert!(commands.is_empty(), "{commands:?}");
    }
}

/// Review R2-T8: the reason is already under the cursor, so Enter on an
/// unavailable row toasts a short pointer — not a truncated copy of it.
#[tokio::test]
async fn an_unavailable_action_is_pointed_at_kept_open_and_never_sent() {
    let mut h = harness().await;
    asked_and_answered(&mut h, decision(false)).await;
    h.app_mut().handle_key(Key::Enter);
    assert!(h.app_mut().ac().sessions.resume_decision.is_some());
    let commands = h.drain_commands().await;
    assert!(commands.is_empty(), "{commands:?}");
    let notes = h.app_mut().notifications.messages();
    assert_eq!(notes, ["Not available yet — see the reason below"]);
    let frame = h.full_frame();
    assert!(frame.contains("not yet (#2012)"), "the reason: {frame}");
}

#[tokio::test]
async fn an_available_action_sends_identity_action_and_version() {
    let mut h = harness().await;
    asked_and_answered(&mut h, decision(true)).await;
    h.app_mut().handle_key(Key::Enter);
    assert!(h.app_mut().ac().sessions.resume_decision.is_none());
    let sent = resume_commands(&h.drain_commands().await);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0]["session"], "cli:foreign");
    assert_eq!(sent[0]["action"], "open_original");
    assert_eq!(sent[0]["expectedHomeVersion"], "h1-0123456789abcdef");
}

#[tokio::test]
async fn a_cancelled_acknowledgement_changes_nothing_and_toasts_nothing() {
    let mut h = harness().await;
    let before = h.full_frame();
    h.app_mut().handle_response(
        Some("resume-x".into()),
        "resume_session".into(),
        true,
        Some(json!({"outcome": "cancelled", "session": "cli:foreign"})),
        None,
    );
    let commands = h.drain_commands().await;
    assert!(commands.is_empty(), "{commands:?}");
    assert_eq!(h.full_frame(), before);
}

#[tokio::test]
async fn a_typed_refusal_is_a_toast_not_a_dialog() {
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
async fn switching_tabs_closes_the_dialog() {
    let mut h = harness().await;
    asked_and_answered(&mut h, decision(false)).await;
    h.app_mut().close_tab_switch_overlays();
    assert!(h.app_mut().ac().sessions.resume_decision.is_none());
}

#[path = "app_resume_answer_tests.rs"]
mod answers;

/// Review R2-T7: the dialog names the session by the title the picker showed
/// for it, with the key beneath; a key that was never listed shows the key.
#[tokio::test]
async fn the_dialog_repeats_the_title_the_picker_showed() {
    let mut h = harness().await;
    h.app_mut().ac_mut().agent_connected = true;
    let data = json!({"sessions": [{
        "name": "hello from A", "key": "cli:foreign", "messageCount": 3,
        "resumeEligible": false, "homeVersion": "h1-0123456789abcdef",
        "executionPath": "/work/other",
    }]});
    h.app_mut().open_resume_selector(&data);
    let picker = h.full_frame();
    assert!(picker.contains("hello from A"), "{picker}");
    assert!(
        picker.contains("Saved in another folder — Enter"),
        "{picker}"
    );
    h.app_mut().handle_key(Key::Enter);
    let id = h.app_mut().ac().pending_session_resume_id.clone().unwrap();
    let _ = h.drain_commands().await;
    h.app_mut().handle_response(
        Some(id),
        "resume_session".into(),
        false,
        Some(decision(false)),
        None,
    );
    let frame = h.full_frame();
    let title = frame.find("hello from A").expect("the picked title");
    assert!(
        title < frame.find("cli:foreign").expect("the key"),
        "{frame}"
    );
    // Re-listing forgets the titles with the versions.
    h.app_mut().handle_key(Key::Escape);
    h.app_mut().send_list_sessions();
    assert!(h.app_mut().ac().sessions.listed_titles.is_empty());
}

/// Review R2-T3: the dialog is laid over the WHOLE frame — on a 40-column
/// terminal it is 36 columns wide, not the ten the body pane would leave it.
#[tokio::test]
async fn the_dialog_spans_the_terminal_not_the_body_pane() {
    for (width, panel) in [(40_usize, 36_usize), (80, 76), (200, 88)] {
        let mut h = super::super::tui_harness::TuiHarness::sized(width, 20).await;
        asked_and_answered(&mut h, decision(false)).await;
        let frame = h.full_frame();
        let top = frame
            .lines()
            .find(|line| line.contains('┌'))
            .expect("a box");
        let border = top.chars().filter(|ch| "┌─┐".contains(*ch)).count();
        assert_eq!(border, panel, "{width}: {frame}");
        assert!(frame.contains("This session belongs to another"), "{frame}");
        for line in frame.lines() {
            assert!(
                crate::components::utils::visible_width(line) <= width,
                "{width}: {line:?}"
            );
        }
    }
}
