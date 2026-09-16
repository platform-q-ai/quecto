use super::{newest_window, page_of};
use crate::application::sessions::dto::{HistoryError, HistoryQuery};
use crate::domain::ids::MessageId;
use crate::domain::message::Message;

fn messages(n: usize) -> Vec<Message> {
    (0..n).map(|i| Message::user(format!("m{i}"))).collect()
}

#[test]
fn the_newest_window_is_the_cursorless_page() {
    let conversation = messages(5);
    let newest = newest_window(&conversation, "", 3);
    let paged = page_of(&conversation, "", &HistoryQuery::newest(3)).unwrap();
    assert_eq!(
        newest.messages.iter().map(|m| m.id()).collect::<Vec<_>>(),
        paged.messages.iter().map(|m| m.id()).collect::<Vec<_>>()
    );
    assert_eq!(newest.before, paged.before);
    assert_eq!(
        newest.before,
        Some(MessageId::from(conversation[2].id().to_string()))
    );
    assert!(newest.has_more_before);
    let empty = newest_window(&conversation, "", 0);
    assert!(empty.messages.is_empty());
    assert!(!empty.has_more_before);
    assert_eq!(empty.before, None);
}

#[test]
fn the_newest_window_hides_the_injected_prompt() {
    let mut conversation = vec![Message::system("injected")];
    conversation.extend(messages(2));
    let newest = newest_window(&conversation, "injected", 64);
    assert_eq!(newest.messages.len(), 2);
    assert!(!newest.has_more_before);
}

#[test]
fn an_unknown_cursor_is_refused_before_paging() {
    let conversation = messages(2);
    let query = HistoryQuery {
        count: 1,
        before: Some(MessageId::from("00000000-0000-0000-0000-000000000000")),
    };
    assert!(matches!(
        page_of(&conversation, "", &query),
        Err(HistoryError::UnknownCursor(_))
    ));
}
