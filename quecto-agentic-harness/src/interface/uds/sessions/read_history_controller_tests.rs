use super::ReadHistoryController;
use crate::application::sessions::active_session::ActiveSessionState;
use crate::application::sessions::dto::HistoryError;
use crate::application::sessions::ports::SessionStore;
use crate::application::sessions::use_cases::ReadHistory;
use crate::domain::message::Message;
use crate::domain::session::Session;
use crate::domain::session_identity::SessionIdentity;
use crate::infrastructure::persistence::session_layout::FlatSessionLayout;
use crate::infrastructure::persistence::session_store::FileSessionStore;
use std::sync::Arc;

fn controller(base: &std::path::Path, published: &[Message]) -> ReadHistoryController {
    let store: Arc<dyn SessionStore> =
        Arc::new(FileSessionStore::new(FlatSessionLayout::new(base)));
    let mut state = ActiveSessionState::new(SessionIdentity::ephemeral());
    state.publish(published);
    ReadHistoryController::new(Arc::new(ReadHistory::new(
        Arc::new(tokio::sync::RwLock::new(state)),
        store,
    )))
}

fn messages(n: usize) -> Vec<Message> {
    (0..n).map(|i| Message::user(format!("m{i}"))).collect()
}

#[test]
fn get_messages_pages_the_resolved_count_before_a_cursor() {
    let tmp = tempfile::tempdir().unwrap();
    let controller = controller(tmp.path(), &[]);
    let conversation = messages(12);
    let page = controller.page(&conversation, "", 8, None).unwrap();
    assert_eq!(page.messages.len(), 8);
    assert!(page.has_more_before);
    let cursor = conversation[10].id().to_string();
    let before = controller
        .page(&conversation, "", 2, Some(&cursor))
        .unwrap();
    assert_eq!(before.messages[1].content, "m9");
    assert!(matches!(
        controller.page(&conversation, "", 8, Some("stale")),
        Err(HistoryError::UnknownCursor(_))
    ));
}

#[test]
fn the_tail_alias_is_the_newest_count_and_newest_page_of_pages_a_published_view() {
    let tmp = tempfile::tempdir().unwrap();
    let controller = controller(tmp.path(), &[]);
    let conversation = messages(5);
    let tail = controller.tail(&conversation, "", 2).unwrap();
    let contents: Vec<_> = tail.messages.iter().map(|m| m.content.as_str()).collect();
    assert_eq!(contents, ["m3", "m4"]);
    assert!(
        controller
            .tail(&conversation, "", 0)
            .unwrap()
            .messages
            .is_empty()
    );
    let newest = controller.newest_page_of(&messages(9), 4);
    assert_eq!(newest.messages.len(), 4);
    assert!(newest.has_more_before);
}

#[tokio::test]
async fn newest_live_page_and_persisted_page_read_the_state_and_the_store() {
    let tmp = tempfile::tempdir().unwrap();
    let controller = controller(tmp.path(), &messages(3));
    let live = controller.newest_live_page(64).await;
    assert_eq!(live.messages.len(), 3);
    assert!(!live.has_more_before);

    let store: Arc<dyn SessionStore> =
        Arc::new(FileSessionStore::new(FlatSessionLayout::new(tmp.path())));
    let mut session = Session::new(SessionIdentity::from_persisted_key("child"));
    session.messages = messages(4);
    store.save(&session).await.unwrap();
    let whole = controller
        .persisted_page("child", None, None)
        .await
        .unwrap();
    assert_eq!(whole.messages.len(), 4);
    let counted = controller
        .persisted_page("child", Some(1), None)
        .await
        .unwrap();
    assert_eq!(counted.messages[0].content, "m3");
    assert!(counted.has_more_before);
    // Message ids are regenerated on every load, so a cursor from a prior
    // page of a persisted transcript never matches: refused, never restarted.
    assert!(matches!(
        controller
            .persisted_page("child", None, Some("stale"))
            .await,
        Err(HistoryError::UnknownCursor(_))
    ));
    assert!(matches!(
        controller.persisted_page("absent", None, None).await,
        Err(HistoryError::TranscriptNotFound)
    ));
    assert!(format!("{controller:?}").starts_with("ReadHistoryController"));
}
