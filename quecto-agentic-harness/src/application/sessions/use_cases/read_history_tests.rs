use super::ReadHistory;
use crate::application::sessions::active_session::ActiveSessionState;
use crate::application::sessions::dto::{HistoryError, HistoryQuery};
use crate::application::sessions::ports::SessionStore;
use crate::domain::ids::MessageId;
use crate::domain::message::Message;
use crate::domain::session::Session;
use crate::domain::session_identity::SessionIdentity;
use crate::infrastructure::persistence::session_layout::FlatSessionLayout;
use crate::infrastructure::persistence::session_store::FileSessionStore;
use std::sync::Arc;

fn read_history(base: &std::path::Path) -> ReadHistory {
    let store: Arc<dyn SessionStore> =
        Arc::new(FileSessionStore::new(FlatSessionLayout::new(base)));
    ReadHistory::new(
        Arc::new(tokio::sync::RwLock::new(ActiveSessionState::new(
            SessionIdentity::ephemeral(),
        ))),
        store,
    )
}

fn messages(n: usize) -> Vec<Message> {
    (0..n).map(|i| Message::user(format!("m{i}"))).collect()
}

fn id(message: &Message) -> MessageId {
    MessageId::from(message.id().to_string())
}

#[test]
fn newest_window_is_chronological_with_a_cursor_only_when_older_history_remains() {
    let tmp = tempfile::tempdir().unwrap();
    let history = read_history(tmp.path());
    let conversation = messages(5);
    let page = history
        .page_of(&conversation, "", &HistoryQuery::newest(3))
        .unwrap();
    let contents: Vec<_> = page.messages.iter().map(|m| m.content.as_str()).collect();
    assert_eq!(contents, ["m2", "m3", "m4"]);
    assert!(page.has_more_before);
    assert_eq!(page.before, Some(id(&conversation[2])));

    let all = history
        .page_of(&conversation, "", &HistoryQuery::newest(80))
        .unwrap();
    assert_eq!(
        all.messages.len(),
        5,
        "an explicit count is not page-clamped"
    );
    assert!(!all.has_more_before);
    assert_eq!(all.before, None);

    let empty = history.page_of(&[], "", &HistoryQuery::newest(5)).unwrap();
    assert!(empty.messages.is_empty());
    assert!(!empty.has_more_before);
}

#[test]
fn count_zero_is_the_empty_page_without_a_cursor() {
    let tmp = tempfile::tempdir().unwrap();
    let history = read_history(tmp.path());
    let page = history
        .page_of(&messages(3), "", &HistoryQuery::newest(0))
        .unwrap();
    assert!(page.messages.is_empty());
    assert!(!page.has_more_before);
    assert_eq!(page.before, None);
}

#[test]
fn a_cursor_pages_strictly_before_it_and_an_unknown_cursor_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let history = read_history(tmp.path());
    let conversation = messages(6);
    let query = HistoryQuery {
        count: 2,
        before: Some(id(&conversation[4])),
    };
    let page = history.page_of(&conversation, "", &query).unwrap();
    let contents: Vec<_> = page.messages.iter().map(|m| m.content.as_str()).collect();
    assert_eq!(contents, ["m2", "m3"]);
    assert_eq!(page.before, Some(id(&conversation[2])));

    let oldest = history
        .page_of(
            &conversation,
            "",
            &HistoryQuery {
                count: 10,
                before: Some(id(&conversation[2])),
            },
        )
        .unwrap();
    assert_eq!(oldest.messages.len(), 2);
    assert!(!oldest.has_more_before);

    for stale in ["not-a-uuid", "00000000-0000-0000-0000-000000000000"] {
        let err = history
            .page_of(
                &conversation,
                "",
                &HistoryQuery {
                    count: 2,
                    before: Some(MessageId::from(stale)),
                },
            )
            .unwrap_err();
        assert!(matches!(&err, HistoryError::UnknownCursor(c) if c.as_str() == stale));
        assert_eq!(
            err.to_string(),
            format!("history cursor not found: {stale}")
        );
    }
}

#[test]
fn the_injected_prompt_is_hidden_and_a_cursor_naming_it_pages_from_the_newest() {
    let tmp = tempfile::tempdir().unwrap();
    let history = read_history(tmp.path());
    let prompt = Message::system("be brief");
    let mut conversation = vec![prompt.clone()];
    conversation.extend(messages(3));
    let page = history
        .page_of(&conversation, "be brief", &HistoryQuery::newest(10))
        .unwrap();
    assert_eq!(page.messages.len(), 3);
    assert!(page.messages.iter().all(|m| m.content != "be brief"));

    let page = history
        .page_of(
            &conversation,
            "be brief",
            &HistoryQuery {
                count: 2,
                before: Some(id(&prompt)),
            },
        )
        .unwrap();
    let contents: Vec<_> = page.messages.iter().map(|m| m.content.as_str()).collect();
    assert_eq!(contents, ["m1", "m2"]);
}

#[tokio::test]
async fn page_live_reads_the_published_transcript() {
    let tmp = tempfile::tempdir().unwrap();
    let history = read_history(tmp.path());
    let page = history.page_live(&HistoryQuery::newest(4)).await.unwrap();
    assert!(page.messages.is_empty());
}

#[tokio::test]
async fn page_persisted_reads_the_store_and_defaults_to_the_whole_transcript() {
    let tmp = tempfile::tempdir().unwrap();
    let history = read_history(tmp.path());
    let store: Arc<dyn SessionStore> =
        Arc::new(FileSessionStore::new(FlatSessionLayout::new(tmp.path())));
    let mut session = Session::new(SessionIdentity::from_persisted_key("child-uuid"));
    session.messages = messages(70);
    store.save(&session).await.unwrap();

    let whole = history
        .page_persisted("child-uuid", None, None)
        .await
        .unwrap();
    assert_eq!(whole.messages.len(), 70);
    assert!(!whole.has_more_before);

    let tail = history
        .page_persisted("child-uuid", Some(3), None)
        .await
        .unwrap();
    assert_eq!(tail.messages.len(), 3);
    assert_eq!(tail.messages[2].content, "m69");
    assert!(tail.has_more_before);

    let err = history
        .page_persisted("child-uuid", None, Some(&MessageId::from("nope")))
        .await
        .unwrap_err();
    assert!(matches!(err, HistoryError::UnknownCursor(_)));

    let err = history
        .page_persisted("absent", None, None)
        .await
        .unwrap_err();
    assert!(matches!(err, HistoryError::TranscriptNotFound));
    assert_eq!(err.to_string(), "no persisted transcript");
}

#[test]
fn debug_output_is_opaque() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(format!("{:?}", read_history(tmp.path())).starts_with("ReadHistory"));
}
