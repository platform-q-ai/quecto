//! What a `resume_session` answer may change in the shell (#2011 review
//! R1-T1, R1-T8, R1-T9, R1-T10): only an affirmative restore changes anything
//! locally; an unreadable success, a peer's refusal and an uncorrelated parse
//! error change nothing; Ctrl-C leaves the dialog sending nothing.
use super::*;

/// Everything a non-restore must leave alone.
fn local_state(
    h: &mut super::super::super::tui_harness::TuiHarness,
) -> (Option<String>, String, usize) {
    let app = h.app_mut();
    let chat = app.ac_mut().master_session.chat.render(120).join("\n");
    (
        app.ac().session_key.clone(),
        chat,
        app.ac().durability_writes,
    )
}

/// Review R1-T1: an owned success whose outcome is not `resumed` — a later
/// slice's, a contradictory one, garbage — adopts no key, resets no clock,
/// writes no manifest, fetches nothing and never says "Resumed".
#[tokio::test]
async fn a_success_that_is_not_a_restore_changes_nothing_locally() {
    for outcome in [
        json!("opened_elsewhere"),
        json!("forked"),
        json!("decision"),
        json!("refused"),
        json!(7),
    ] {
        let mut h = harness().await;
        h.app_mut().ac_mut().agent_connected = true;
        h.app_mut().ac_mut().session_key = Some("cli:local".into());
        h.app_mut().send_resume_session("cli:foreign");
        let id = h.app_mut().ac().pending_session_resume_id.clone().unwrap();
        let _ = h.drain_commands().await;
        let before = local_state(&mut h);
        h.app_mut().handle_response(
            Some(id),
            "resume_session".into(),
            true,
            Some(
                json!({"outcome": outcome, "session": "cli:foreign", "sessionKey": "cli:foreign"}),
            ),
            None,
        );
        assert_eq!(local_state(&mut h), before, "{outcome}");
        assert!(
            h.app_mut().ac().pending_session_resume_id.is_none(),
            "settled"
        );
        let commands = h.drain_commands().await;
        assert!(
            commands.is_empty(),
            "{outcome}: nothing is re-fetched: {commands:?}"
        );
        let notes = h.app_mut().notifications.messages();
        assert!(
            !notes.iter().any(|note| note.contains("Resumed")),
            "{notes:?}"
        );
        assert!(
            notes
                .iter()
                .any(|note| note.contains("does not understand")
                    && note.contains("nothing was changed")),
            "{outcome}: a plain message: {notes:?}"
        );
    }
}

/// The affirmative forms still restore: `resumed`, and a pre-#2011 harness
/// that names no outcome.
#[tokio::test]
async fn resumed_and_the_legacy_shape_adopt_the_key() {
    for data in [
        json!({"outcome": "resumed", "session": "a", "sessionKey": "cli:a"}),
        json!({"session": "a", "sessionKey": "cli:a"}),
    ] {
        let mut h = harness().await;
        h.app_mut().handle_response(
            Some("r".into()),
            "resume_session".into(),
            true,
            Some(data),
            None,
        );
        assert_eq!(h.app_mut().ac().session_key.as_deref(), Some("cli:a"));
        let notes = h.app_mut().notifications.messages();
        assert!(
            notes.iter().any(|note| note.contains("Resumed session a")),
            "{notes:?}"
        );
    }
}

/// The untrusted outcome text cannot carry controls into the toast.
#[tokio::test]
async fn an_unreadable_outcome_is_shown_safely() {
    let mut h = harness().await;
    asked_and_answered_with(
        &mut h,
        true,
        json!({"outcome": "x\u{1b}[2J\u{202e}y".repeat(30)}),
    )
    .await;
    let notes = h.app_mut().notifications.messages().join("\n");
    assert!(
        !notes.contains('\u{1b}') && !notes.contains('\u{202e}'),
        "{notes:?}"
    );
}

async fn asked_and_answered_with(
    h: &mut super::super::super::tui_harness::TuiHarness,
    success: bool,
    data: serde_json::Value,
) {
    h.app_mut().ac_mut().agent_connected = true;
    h.app_mut().send_resume_session("cli:foreign");
    let id = h.app_mut().ac().pending_session_resume_id.clone().unwrap();
    let _ = h.drain_commands().await;
    h.app_mut().handle_response(
        Some(id),
        "resume_session".into(),
        success,
        Some(data),
        Some("refused".into()),
    );
}

/// Review R1-T9: another tab's refusal (or unreadable answer) is not this
/// tab's — like another tab's decision. An answer with no id at all is
/// nobody's in particular and its failure is still toasted (pinned before).
#[tokio::test]
async fn a_peers_refusal_toasts_nothing_here() {
    let mut h = harness().await;
    let before = h.app_mut().notifications.messages().len();
    for (success, data) in [
        (false, json!({"outcome": "refused", "code": "not_found"})),
        (true, json!({"outcome": "forked"})),
    ] {
        h.app_mut().handle_response(
            Some("tab9:resume-1".into()),
            "resume_session".into(),
            success,
            Some(data),
            Some("session not found: x".into()),
        );
    }
    assert_eq!(h.app_mut().notifications.messages().len(), before);
    h.app_mut().handle_response(
        None,
        "resume_session".into(),
        false,
        None,
        Some("err".into()),
    );
    assert_eq!(h.app_mut().notifications.messages().len(), before + 1);
}

/// Review R1-T8 / R1-H11: an action the harness does not know is answered by
/// the UNCORRELATED `parse_error` — no id will ever settle the request. The
/// TUI settles it on that error, changes nothing and says so; an unrelated
/// parse error, or one with no resume in flight, touches nothing.
#[tokio::test]
async fn an_uncorrelated_parse_error_settles_the_resume_in_flight() {
    let mut h = harness().await;
    let unknown_action = Some("unknown resume action at line 1 column 80".to_string());
    h.app_mut().handle_response(
        None,
        "parse_error".into(),
        false,
        None,
        unknown_action.clone(),
    );
    assert!(
        h.app_mut().notifications.messages().is_empty(),
        "nothing in flight"
    );
    h.app_mut().ac_mut().agent_connected = true;
    h.app_mut().ac_mut().session_key = Some("cli:local".into());
    h.app_mut().send_resume_session("cli:foreign");
    let _ = h.drain_commands().await;
    let before = local_state(&mut h);
    let unrelated = Some("missing field `message` at line 1 column 20".to_string());
    h.app_mut()
        .handle_response(None, "parse_error".into(), false, None, unrelated);
    assert!(
        h.app_mut().ac().pending_session_resume_id.is_some(),
        "not a resume's"
    );
    h.app_mut()
        .handle_response(None, "parse_error".into(), false, None, unknown_action);
    assert!(
        h.app_mut().ac().pending_session_resume_id.is_none(),
        "settled"
    );
    assert_eq!(local_state(&mut h), before);
    let notes = h.app_mut().notifications.messages();
    assert!(
        notes.iter().any(|note| note.contains("Resume failed")),
        "{notes:?}"
    );
}

/// Review R1-T10: Ctrl-C in the dialog is Escape — it closes, sends nothing
/// (no abort either) and changes nothing.
#[tokio::test]
async fn ctrl_c_closes_the_dialog_and_sends_nothing() {
    let mut h = harness().await;
    asked_and_answered(&mut h, decision(true)).await;
    let before = local_state(&mut h);
    h.app_mut().handle_key(Key::Ctrl('c'));
    assert!(h.app_mut().ac().sessions.resume_decision.is_none());
    let commands = h.drain_commands().await;
    assert!(commands.is_empty(), "{commands:?}");
    assert_eq!(local_state(&mut h), before);
}

/// Review R1-T4: a scope switch drops the versions of the rows it replaces,
/// so a row still on screen cannot be sent with a version of the old listing.
#[tokio::test]
async fn a_scope_switch_clears_the_listed_versions() {
    let mut h = harness().await;
    h.app_mut().ac_mut().agent_connected = true;
    let data = json!({"sessions": [{
        "key": "chat-1-else", "title": "t", "messageCount": 3,
        "resumeEligible": true, "homeVersion": "h1-00000000000000aa",
    }]});
    let manifest = std::env::temp_dir().join("s2011f1-no-manifest.json");
    h.app_mut().open_resume_selector_at(&data, &manifest);
    assert_eq!(h.app_mut().ac().sessions.home_versions.len(), 1);
    h.app_mut()
        .request_session_scope(crate::protocol::session_payloads::SessionListScope::Global);
    assert!(h.app_mut().ac().sessions.home_versions.is_empty());
    let _ = h.drain_commands().await;
    h.app_mut().handle_key(Key::Enter);
    let sent = resume_commands(&h.drain_commands().await);
    assert!(
        sent.iter()
            .all(|command| command.get("expectedHomeVersion").is_none()),
        "{sent:?}"
    );
}

/// A success with no payload at all (nothing to read an identity from) still
/// reports a restore and refreshes, and adopts no key.
#[tokio::test]
async fn a_bare_success_reports_a_restore_without_adopting_a_key() {
    let mut h = harness().await;
    h.app_mut().ac_mut().session_key = Some("cli:local".into());
    h.app_mut()
        .handle_response(Some("r".into()), "resume_session".into(), true, None, None);
    assert_eq!(h.app_mut().ac().session_key.as_deref(), Some("cli:local"));
    let notes = h.app_mut().notifications.messages();
    assert!(
        notes.iter().any(|note| note == "Resumed session session"),
        "{notes:?}"
    );
}

/// A peer tab's quiet footer refresh stays quiet; a solicited `/session`
/// answer is shown (the sessions controller's stats routing).
#[tokio::test]
async fn a_peers_quiet_stats_refresh_is_dropped_and_a_solicited_one_is_shown() {
    let mut h = harness().await;
    let data = json!({"sessionKey": "cli:s", "totalMessages": 1});
    let chat = |h: &mut super::super::super::tui_harness::TuiHarness| {
        h.app_mut()
            .ac_mut()
            .master_session
            .chat
            .render(120)
            .join("\n")
    };
    let before = chat(&mut h);
    h.app_mut().handle_response(
        Some("tab9:stats-footer".into()),
        "get_session_stats".into(),
        true,
        Some(data.clone()),
        None,
    );
    assert_eq!(chat(&mut h), before, "a peer's quiet refresh adds nothing");
    h.app_mut().handle_response(
        Some("session-1".into()),
        "get_session_stats".into(),
        true,
        Some(data),
        None,
    );
    assert!(
        chat(&mut h).contains("cli:s"),
        "a solicited answer is shown"
    );
}
