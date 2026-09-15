//! #1060 lifecycle regressions split out to keep dispatch coverage under file cap.

use super::fixture_tests::Fixture;
use super::{handle_clear_history, handle_new_session, handle_resume_session, handle_rewind_to};
use crate::application::sessions::ports::SessionStore;
use crate::domain::message::Message;
use crate::domain::session::Session;

#[tokio::test]
async fn clear_history_clears_message_ref_lookup_ledger() {
    let mut fx = Fixture::new();
    let msg = Message::assistant("secret answer", vec![]);
    let msg_id = msg.id().to_string();
    fx.messages.push(msg.clone());

    let snapshot = {
        let ctx = fx.ctx();
        let snapshot = ctx.sessions.clone();
        snapshot.active_session.write().await.record_full(&[msg]);
        assert!(
            snapshot
                .active_session
                .read()
                .await
                .conversation()
                .lookup(&msg_id)
                .is_some()
        );
        snapshot
    };

    {
        let mut ctx = fx.ctx();
        ctx.sessions = snapshot.clone();
        assert!(!handle_clear_history(&mut ctx, None, "clear_history").await);
    }

    assert!(fx.messages.is_empty());
    let snap = snapshot.active_session.read().await;
    assert!(
        snap.conversation().live_messages().is_empty(),
        "live snapshot should be cleared"
    );
    assert!(
        snap.conversation().lookup(&msg_id).is_none(),
        "old message ref must not remain fetchable after clear_history"
    );
}

#[tokio::test]
async fn new_session_clears_message_ref_lookup_ledger() {
    let mut fx = Fixture::new();
    let msg = Message::assistant("secret answer", vec![]);
    let msg_id = msg.id().to_string();
    fx.messages.push(msg.clone());

    let snapshot = {
        let ctx = fx.ctx();
        let snapshot = ctx.sessions.clone();
        snapshot.active_session.write().await.record_full(&[msg]);
        assert!(
            snapshot
                .active_session
                .read()
                .await
                .conversation()
                .lookup(&msg_id)
                .is_some()
        );
        snapshot
    };
    {
        let mut ctx = fx.ctx();
        ctx.sessions = snapshot.clone();
        assert!(!handle_new_session(&mut ctx, None, "new_session").await);
    }
    assert!(
        snapshot
            .active_session
            .read()
            .await
            .conversation()
            .lookup(&msg_id)
            .is_none(),
        "old ref must not remain fetchable after /new"
    );
}

#[tokio::test]
async fn resume_session_clears_previous_session_ref() {
    let mut fx = Fixture::new();
    let old = Message::assistant("previous session secret", vec![]);
    let old_id = old.id().to_string();
    fx.messages.push(old.clone());

    // Pre-save a DIFFERENT session to resume into.
    let key = Session::build_key("cli", "saved");
    fx.store
        .save(&Session {
            key: crate::domain::session_identity::SessionIdentity::from_persisted_key(key),
            messages: vec![Message::user("restored")],
            workflow_run: None,
            subagent_roster: Vec::new(),
        })
        .await
        .unwrap();

    let snapshot = {
        let ctx = fx.ctx();
        let snapshot = ctx.sessions.clone();
        snapshot.active_session.write().await.record_full(&[old]);
        assert!(
            snapshot
                .active_session
                .read()
                .await
                .conversation()
                .lookup(&old_id)
                .is_some()
        );
        snapshot
    };
    {
        let mut ctx = fx.ctx();
        ctx.sessions = snapshot.clone();
        assert!(
            !handle_resume_session(&mut ctx, Some("rs"), "resume_session", "saved".into()).await
        );
    }
    assert!(
        snapshot
            .active_session
            .read()
            .await
            .conversation()
            .lookup(&old_id)
            .is_none(),
        "a ref from the PREVIOUS session must not resolve after resume"
    );
}

#[tokio::test]
async fn rewind_to_drops_rewound_away_message_ref() {
    let mut fx = Fixture::new();
    let keep = Message::assistant("kept answer", vec![]);
    let drop = Message::assistant("rewound-away answer", vec![]);
    let (keep_id, drop_id) = (keep.id().to_string(), drop.id().to_string());
    // [user0, keep(assistant1), user2, drop(assistant3)]; rewind to user2 (idx 2)
    // truncates to [user0, keep].
    fx.messages.push(Message::user("first"));
    fx.messages.push(keep.clone());
    fx.messages.push(Message::user("second"));
    fx.messages.push(drop.clone());

    let snapshot = {
        let ctx = fx.ctx();
        let snapshot = ctx.sessions.clone();
        snapshot
            .active_session
            .write()
            .await
            .record_full(&[keep, drop]);
        snapshot
    };
    {
        let mut ctx = fx.ctx();
        ctx.sessions = snapshot.clone();
        assert!(!handle_rewind_to(&mut ctx, Some("r"), "rewind_to", Some(2), None).await);
    }
    let snap = snapshot.active_session.read().await;
    assert!(
        snap.conversation().lookup(&drop_id).is_none(),
        "a rewound-away message ref must not remain fetchable"
    );
    assert!(
        snap.conversation().lookup(&keep_id).is_some(),
        "a surviving message must still resolve after rewind"
    );
}

// ─── D6 (#1975): the streaming refusals are admission, pinned on the wire ──

/// Drive `handler` with the agent streaming over a two-turn conversation
/// and a valid user target; returns the events the handler emitted.
async fn refused_while_streaming(
    handler: impl AsyncFnOnce(&mut crate::interface::cli::uds::DispatchCtx<'_>) -> bool,
) -> Vec<serde_json::Value> {
    let mut fx = Fixture::new();
    fx.messages.push(Message::user("first"));
    fx.messages.push(Message::assistant("answer", vec![]));
    fx.messages.push(Message::user("second"));
    let before: Vec<String> = fx.messages.iter().map(|m| m.content.clone()).collect();
    fx.session.set_streaming(true);
    let (tx, mut rx) = tokio::sync::broadcast::channel(16);
    {
        let mut ctx = fx.ctx();
        ctx.broadcast_tx = Some(tx);
        assert!(!handler(&mut ctx).await);
    }
    let mut events = Vec::new();
    while let Ok(line) = rx.try_recv() {
        events.push(serde_json::from_str(line.trim()).unwrap());
    }
    let after: Vec<String> = fx.messages.iter().map(|m| m.content.clone()).collect();
    assert_eq!(after, before, "a refused command changes nothing");
    events
}

#[tokio::test]
async fn clear_history_while_streaming_answers_the_refusal_and_touches_nothing() {
    let events = refused_while_streaming(async |ctx| {
        handle_clear_history(ctx, Some("c"), "clear_history").await
    })
    .await;
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0]["type"], "response");
    assert_eq!(events[0]["success"], false);
    assert_eq!(
        events[0]["error"],
        "cannot clear history while agent is running"
    );
}

#[tokio::test]
async fn rewind_to_while_streaming_answers_the_refusal_and_touches_nothing() {
    let events = refused_while_streaming(async |ctx| {
        let target = ctx.messages[2].id().to_string();
        handle_rewind_to(ctx, Some("r"), "rewind_to", None, Some(target)).await
    })
    .await;
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0]["type"], "response");
    assert_eq!(events[0]["success"], false);
    assert_eq!(events[0]["error"], "cannot rewind while agent is running");
}
