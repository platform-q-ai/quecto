use super::cov_tests::Fixture;
use super::persist_current_session;
use crate::domain::message::Message;
use crate::domain::session::SessionStore;
use crate::interface::cli::uds_session::{HISTORY_PAGE_SIZE, messages_page_json};

#[tokio::test]
async fn same_process_persist_then_prune_keeps_live_ordinals_durable_and_monotonic() {
    let mut fx = Fixture::new();
    let mut old_user = Message::user("old-user");
    old_user.ordinal = Some(40);
    let mut old_assistant = Message::assistant("old-assistant", vec![]);
    old_assistant.ordinal = Some(41);
    fx.messages = vec![old_user, old_assistant, Message::user("new-before-prune")];

    {
        let mut ctx = fx.ctx();
        persist_current_session(&mut ctx).await.unwrap();
        assert_eq!(ctx.messages[2].ordinal, Some(42));
        ctx.messages.remove(0);
        ctx.durable_prefix_dirty = true;
        ctx.messages
            .push(Message::assistant("new-after-prune", vec![]));
        persist_current_session(&mut ctx).await.unwrap();
        assert_eq!(ctx.messages[2].ordinal, Some(43));
    }

    let loaded = fx.store.load("cli:test").await.unwrap().unwrap();
    assert_eq!(
        loaded
            .messages
            .iter()
            .map(|m| m.ordinal)
            .collect::<Vec<_>>(),
        vec![Some(41), Some(42), Some(43)]
    );
    assert_eq!(
        messages_page_json(&fx.messages, HISTORY_PAGE_SIZE, None)["messages"][2]["ordinal"],
        43
    );
}

#[test]
fn unpersisted_history_never_fabricates_an_ordinal_from_vector_position() {
    let mut durable = Message::user("durable");
    durable.ordinal = Some(100);
    let pending = Message::assistant("current report", vec![]);
    let page = messages_page_json(&[durable, pending.clone()], HISTORY_PAGE_SIZE, None);
    assert_eq!(page["messages"][1]["ordinal"], serde_json::Value::Null);
    assert_eq!(page["messages"][1]["id"], pending.id().to_string());
}
