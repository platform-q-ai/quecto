//! #1060 lifecycle regressions split out to keep dispatch coverage under file cap.

use super::cov_tests::Fixture;
use super::handle_clear_history;
use crate::domain::message::Message;

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
