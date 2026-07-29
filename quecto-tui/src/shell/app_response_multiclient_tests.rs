//! Multi-client correlation characterization for solicited get_messages (#1237).
//!
//! `get_messages` responses are broadcast to every UDS client. Fixed literal
//! request ids let Client A's resume/rewind/attach clobber Client B. Dual
//! harnesses simulate two logical TUI clients without two real sockets.

use super::app_paged_history_tests::{chat_text, harness, page, respond, widen_active_viewport};
use super::tui_harness::TuiHarness;
use super::*;

fn raw_respond(app: &mut App, id: Option<&str>, data: serde_json::Value) {
    app.handle_response(
        id.map(String::from),
        "get_messages".into(),
        true,
        Some(data),
        None,
    );
}

fn seed_client_b_transcript(app: &mut App) {
    app.master_session.chat.add_entry(ChatEntry::User {
        text: "client-b private turn".into(),
    });
    app.master_session.history.before_cursor = Some("b-cursor".into());
    app.master_session.history.has_more_before = true;
    app.master_session.history.partial_prefix_len = Some(1);
}

fn b_frame(app: &mut App) -> String {
    widen_active_viewport(app);
    chat_text(app)
}

fn legacy_messages(user: &str, assistant: &str) -> serde_json::Value {
    serde_json::json!({
        "messages": [
            {"role": "user", "content": user},
            {"role": "assistant", "content": assistant},
        ]
    })
}

async fn mint_resume_id(h: &mut TuiHarness) -> String {
    h.app_mut().handle_response(
        Some("resume".into()),
        "resume_session".into(),
        true,
        Some(serde_json::json!({"session": "s-a"})),
        None,
    );
    let id = h
        .app_mut()
        .test_pending_resume_messages_id()
        .expect("resume mints pending id")
        .to_string();
    let _ = h.drain_commands().await;
    id
}

async fn mint_rewind_refresh_id(h: &mut TuiHarness) -> String {
    h.app_mut().rewind.pending_apply_id = Some("rw-a".into());
    h.app_mut()
        .handle_response(Some("rw-a".into()), "rewind_to".into(), true, None, None);
    let id = h
        .app_mut()
        .test_pending_rewind_refresh_id()
        .expect("rewind_to mints refresh id")
        .to_string();
    let _ = h.drain_commands().await;
    id
}

async fn mint_attach_id(h: &mut TuiHarness) -> String {
    h.app_mut().request_master_attach_backfill();
    let id = h
        .app_mut()
        .test_pending_attach_backfill_id()
        .expect("attach mints pending id")
        .to_string();
    let _ = h.drain_commands().await;
    id
}

#[tokio::test]
async fn foreign_resume_does_not_replace_other_client_transcript() {
    let (mut a, mut b) = (harness().await, harness().await);
    seed_client_b_transcript(b.app_mut());
    let before_cursor = b.app_mut().master_session.history.before_cursor.clone();
    let before_frame = b_frame(b.app_mut());

    let resume_id = mint_resume_id(&mut a).await;
    // Broadcast of A's solicited resume response reaches B.
    respond(
        b.app_mut(),
        Some(&resume_id),
        "get_messages",
        true,
        legacy_messages("a resumed user", "a resumed assistant"),
    );

    let frame = b_frame(b.app_mut());
    assert_eq!(
        frame, before_frame,
        "foreign resume must not mutate B's transcript:\nbefore:\n{before_frame}\nafter:\n{frame}"
    );
    assert!(
        !frame.contains("Session resumed"),
        "B must not show Session resumed for A's resume:\n{frame}"
    );
    assert_eq!(
        b.app_mut().master_session.history.before_cursor,
        before_cursor,
        "foreign resume must not clobber B paging cursors"
    );
}

#[tokio::test]
async fn foreign_rewind_refresh_does_not_replace_other_client_transcript() {
    let (mut a, mut b) = (harness().await, harness().await);
    seed_client_b_transcript(b.app_mut());
    let before_frame = b_frame(b.app_mut());

    let refresh_id = mint_rewind_refresh_id(&mut a).await;
    respond(
        b.app_mut(),
        Some(&refresh_id),
        "get_messages",
        true,
        legacy_messages("a rewound user", "a rewound assistant"),
    );

    let frame = b_frame(b.app_mut());
    assert_eq!(
        frame, before_frame,
        "foreign rewind-refresh must not mutate B"
    );
    assert!(
        !frame.contains("Conversation rewound"),
        "B must not show Conversation rewound for A's refresh:\n{frame}"
    );
}

#[tokio::test]
async fn own_resume_still_replaces_and_shows_status() {
    let mut a = harness().await;
    a.app_mut().handle_event(Event::Token {
        token: "SHOULD_BE_CLEARED".into(),
    });
    let resume_id = mint_resume_id(&mut a).await;
    respond(
        a.app_mut(),
        Some(&resume_id),
        "get_messages",
        true,
        legacy_messages("own resumed user", "own resumed assistant"),
    );
    let frame = b_frame(a.app_mut());
    assert!(frame.contains("Session resumed"), "{frame}");
    assert!(frame.contains("own resumed user"), "{frame}");
    assert!(!frame.contains("SHOULD_BE_CLEARED"), "{frame}");
}

#[tokio::test]
async fn foreign_same_family_unknown_id_does_not_hit_legacy_replace() {
    let mut b = harness().await;
    seed_client_b_transcript(b.app_mut());
    let before_frame = b_frame(b.app_mut());

    // No pending on B; foreign minted family id must DROP, not fall through.
    respond(
        b.app_mut(),
        Some("resume-messages-foreign-1"),
        "get_messages",
        true,
        legacy_messages("foreign user", "foreign assistant"),
    );
    respond(
        b.app_mut(),
        Some("rewind-refresh-foreign-2"),
        "get_messages",
        true,
        legacy_messages("foreign rw user", "foreign rw assistant"),
    );
    respond(
        b.app_mut(),
        Some("attach-backfill-foreign-3"),
        "get_messages",
        true,
        legacy_messages("foreign attach user", "foreign attach assistant"),
    );
    // Bare legacy literals from old peers are also non-pending → drop.
    // Use raw_respond so the test helper does not arm pending for the bare id.
    raw_respond(
        b.app_mut(),
        Some("resume-messages"),
        legacy_messages("bare resume user", "bare resume assistant"),
    );

    let frame = b_frame(b.app_mut());
    assert_eq!(
        frame, before_frame,
        "foreign same-family ids must not hit legacy replace:\nbefore:\n{before_frame}\nafter:\n{frame}"
    );
    assert!(!frame.contains("Session resumed"), "{frame}");
    assert!(!frame.contains("foreign user"), "{frame}");
    assert!(!frame.contains("bare resume user"), "{frame}");
}

#[tokio::test]
async fn id_less_busy_connect_snapshot_still_reconciles_on_unrelated_client() {
    let mut b = harness().await;
    b.app_mut().handle_event(Event::Token {
        token: "LIVE_B".into(),
    });
    let mut snapshot = legacy_messages("snapshot question", "snapshot answer");
    if let Some(obj) = snapshot.as_object_mut() {
        obj.insert("snapshot".into(), serde_json::json!(true));
    }
    respond(b.app_mut(), None, "get_messages", true, snapshot);
    let frame = b_frame(b.app_mut());
    assert!(
        frame.contains("snapshot answer") && frame.contains("LIVE_B"),
        "id-less snapshot must still reconcile:\n{frame}"
    );
    assert!(!frame.contains("Session resumed"), "{frame}");
}

#[tokio::test]
async fn foreign_attach_backfill_does_not_reconcile_other_client() {
    let (mut a, mut b) = (harness().await, harness().await);
    seed_client_b_transcript(b.app_mut());
    let before_frame = b_frame(b.app_mut());
    let attach_id = mint_attach_id(&mut a).await;

    respond(
        b.app_mut(),
        Some(&attach_id),
        "get_messages",
        true,
        legacy_messages("a attach user", "a attach assistant"),
    );
    let frame = b_frame(b.app_mut());
    assert_eq!(frame, before_frame, "foreign attach must not mutate B");
    assert!(!frame.contains("a attach user"), "{frame}");
}

#[tokio::test]
async fn own_attach_backfill_still_reconciles_and_preserves_live_tokens() {
    let mut a = harness().await;
    a.app_mut().handle_event(Event::Token {
        token: "LIVE_A".into(),
    });
    let attach_id = mint_attach_id(&mut a).await;
    respond(
        a.app_mut(),
        Some(&attach_id),
        "get_messages",
        true,
        legacy_messages("own attach user", "own attach assistant"),
    );
    let frame = b_frame(a.app_mut());
    assert!(frame.contains("own attach user"), "{frame}");
    assert!(frame.contains("LIVE_A"), "{frame}");
    assert!(!frame.contains("Session resumed"), "{frame}");
}

#[tokio::test]
async fn own_rewind_refresh_still_replaces_with_kind_status() {
    let mut a = harness().await;
    a.app_mut().handle_event(Event::Token {
        token: "PRE_REWIND".into(),
    });
    let refresh_id = mint_rewind_refresh_id(&mut a).await;
    respond(
        a.app_mut(),
        Some(&refresh_id),
        "get_messages",
        true,
        page(&[("m1", "post rewind turn")], None, false),
    );
    let frame = b_frame(a.app_mut());
    assert!(frame.contains("Conversation rewound"), "{frame}");
    assert!(frame.contains("post rewind turn"), "{frame}");
    assert!(!frame.contains("PRE_REWIND"), "{frame}");
    assert!(!frame.contains("Session resumed"), "{frame}");
}
