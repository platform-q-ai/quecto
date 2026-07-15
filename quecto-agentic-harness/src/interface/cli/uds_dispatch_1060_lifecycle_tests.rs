//! #1060 lifecycle regressions split out to keep dispatch coverage under file cap.

use super::cov_tests::Fixture;
use super::{handle_clear_history, handle_new_session, handle_resume_session, handle_rewind_to};
use crate::domain::message::Message;
use crate::domain::session::{Session, SessionStore};

#[tokio::test]
async fn clear_history_clears_message_ref_lookup_ledger() {
    let mut fx = Fixture::new();
    let msg = Message::assistant("secret answer", vec![]);
    let msg_id = msg.id().to_string();
    fx.messages.push(msg.clone());

    let snapshot = {
        let ctx = fx.ctx();
        let snapshot = ctx.conversation_snapshot.clone();
        snapshot.write().await.record_full(&[msg]);
        assert!(snapshot.read().await.resolve(&msg_id).is_some());
        snapshot
    };

    {
        let mut ctx = fx.ctx();
        ctx.conversation_snapshot = snapshot.clone();
        assert!(!handle_clear_history(&mut ctx, None, "clear_history").await);
    }

    assert!(fx.messages.is_empty());
    let snap = snapshot.read().await;
    assert!(snap.messages.is_empty(), "live snapshot should be cleared");
    assert!(
        snap.resolve(&msg_id).is_none(),
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
        let snapshot = ctx.conversation_snapshot.clone();
        snapshot.write().await.record_full(&[msg]);
        assert!(snapshot.read().await.resolve(&msg_id).is_some());
        snapshot
    };
    {
        let mut ctx = fx.ctx();
        ctx.conversation_snapshot = snapshot.clone();
        assert!(!handle_new_session(&mut ctx, None, "new_session").await);
    }
    assert!(
        snapshot.read().await.resolve(&msg_id).is_none(),
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
            key,
            messages: vec![Message::user("restored")],
            workflow_run: None,
        })
        .await
        .unwrap();

    let snapshot = {
        let ctx = fx.ctx();
        let snapshot = ctx.conversation_snapshot.clone();
        snapshot.write().await.record_full(&[old]);
        assert!(snapshot.read().await.resolve(&old_id).is_some());
        snapshot
    };
    {
        let mut ctx = fx.ctx();
        ctx.conversation_snapshot = snapshot.clone();
        assert!(
            !handle_resume_session(&mut ctx, Some("rs"), "resume_session", "saved".into()).await
        );
    }
    assert!(
        snapshot.read().await.resolve(&old_id).is_none(),
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
        let snapshot = ctx.conversation_snapshot.clone();
        snapshot.write().await.record_full(&[keep, drop]);
        snapshot
    };
    {
        let mut ctx = fx.ctx();
        ctx.conversation_snapshot = snapshot.clone();
        assert!(!handle_rewind_to(&mut ctx, Some("r"), "rewind_to", Some(2), None).await);
    }
    let snap = snapshot.read().await;
    assert!(
        snap.resolve(&drop_id).is_none(),
        "a rewound-away message ref must not remain fetchable"
    );
    assert!(
        snap.resolve(&keep_id).is_some(),
        "a surviving message must still resolve after rewind"
    );
}
