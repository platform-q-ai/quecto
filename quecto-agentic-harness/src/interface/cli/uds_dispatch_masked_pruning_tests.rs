use super::cov_tests::Fixture;
use super::*;
use crate::domain::{message::Message, session::Session};

#[tokio::test]
async fn persist_reconciles_masked_pruning_that_changed_the_durable_prefix() {
    let mut fx = Fixture::new();
    fx.messages = vec![Message::user("old-a"), Message::assistant("old-b", vec![])];
    fx.store
        .save(&Session {
            key: fx.session_key.clone(),
            messages: fx.messages.clone(),
            workflow_run: None,
        })
        .await
        .unwrap();
    fx.last_persisted_message_index = fx.messages.len();

    fx.messages = vec![
        Message::user("pruned-a"),
        Message::assistant("new-c", vec![]),
    ];
    let mut ctx = fx.ctx();
    ctx.durable_prefix_dirty = true;
    persist_current_session(&mut ctx).await.unwrap();

    let resumed = fx.store.load(&fx.session_key).await.unwrap().unwrap();
    let contents: Vec<_> = resumed
        .messages
        .iter()
        .map(|message| message.content.as_str())
        .collect();
    assert_eq!(contents, ["pruned-a", "new-c"]);
}

// ─── dispatch_command routing ────────────────────────────────────────────────
