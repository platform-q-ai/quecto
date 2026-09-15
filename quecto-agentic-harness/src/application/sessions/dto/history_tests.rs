use super::{HistoryError, HistoryPage, HistoryQuery};
use crate::domain::error::DomainError;
use crate::domain::ids::MessageId;
use crate::domain::message::Message;

fn page(n: usize, has_more_before: bool) -> HistoryPage {
    let messages: Vec<Message> = (0..n).map(|i| Message::user(format!("m{i}"))).collect();
    let before = has_more_before.then(|| MessageId::from(messages[0].id().to_string()));
    HistoryPage {
        messages,
        before,
        has_more_before,
    }
}

#[test]
fn newest_query_has_no_cursor() {
    assert_eq!(
        HistoryQuery::newest(7),
        HistoryQuery {
            count: 7,
            before: None
        }
    );
}

#[test]
fn keeping_newest_drops_the_oldest_and_moves_the_cursor() {
    let trimmed = page(4, false).keeping_newest(2);
    let contents: Vec<_> = trimmed
        .messages
        .iter()
        .map(|m| m.content.as_str())
        .collect();
    assert_eq!(contents, ["m2", "m3"]);
    assert!(
        trimmed.has_more_before,
        "dropped messages are older history"
    );
    assert_eq!(
        trimmed.before,
        Some(MessageId::from(trimmed.messages[0].id().to_string()))
    );
}

#[test]
fn keeping_at_least_the_whole_page_changes_nothing() {
    let original = page(3, true);
    let cursor = original.before.clone();
    let kept = original.clone().keeping_newest(3);
    assert_eq!(kept.messages.len(), 3);
    assert_eq!(kept.before, cursor);
    assert!(kept.has_more_before);
    let kept = original.keeping_newest(10);
    assert_eq!(kept.messages.len(), 3);
    assert_eq!(kept.before, cursor);
}

#[test]
fn keeping_none_of_an_empty_page_reports_no_cursor() {
    let kept = page(0, false).keeping_newest(0);
    assert!(kept.messages.is_empty());
    assert_eq!(kept.before, None);
    assert!(!kept.has_more_before);
}

#[test]
fn errors_display_their_cause() {
    assert_eq!(
        HistoryError::UnknownCursor(MessageId::from("abc")).to_string(),
        "history cursor not found: abc"
    );
    assert_eq!(
        HistoryError::TranscriptNotFound.to_string(),
        "no persisted transcript"
    );
    assert_eq!(
        HistoryError::Store(DomainError::Session("disk".into())).to_string(),
        DomainError::Session("disk".into()).to_string()
    );
}

#[test]
fn keeping_none_of_a_non_empty_page_still_keeps_its_newest_message() {
    let kept = page(3, false).keeping_newest(0);
    assert_eq!(kept.messages.len(), 1);
    assert_eq!(kept.messages[0].content, "m2");
    assert!(kept.has_more_before);
    assert_eq!(
        kept.before,
        Some(MessageId::from(kept.messages[0].id().to_string())),
        "a reported older history always carries its cursor"
    );
}
