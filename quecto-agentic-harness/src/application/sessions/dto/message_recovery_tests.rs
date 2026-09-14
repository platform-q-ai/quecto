use super::{ContentSelector, RecoveryError, Utf8Range};
use crate::domain::ids::{MessageId, ToolCallId};

#[test]
fn whole_message_selector_has_no_range_arguments() {
    assert_eq!(
        ContentSelector::whole_message(),
        ContentSelector::Message {
            offset: None,
            thinking_offset: None,
            limit: None,
        }
    );
}

#[test]
fn requested_range_defaults_to_the_whole_text() {
    let range = Utf8Range::requested("hello", None, None);
    assert_eq!((range.start, range.end), (0, 5));
    assert!(!range.is_empty());
    assert_eq!(range.slice("hello"), "hello");
}

#[test]
fn requested_range_clamps_offset_and_limit_to_character_boundaries() {
    // a(1) é(2) 日(3) z(1): bytes 0..7.
    let text = "aé日z";
    let range = Utf8Range::requested(text, Some(2), Some(4));
    assert_eq!((range.start, range.end), (1, 3));
    assert_eq!(range.slice(text), "é");
    // A limit inside `日` from its start yields the whole character.
    let range = Utf8Range::requested(text, Some(3), Some(1));
    assert_eq!((range.start, range.end), (3, 6));
    assert_eq!(range.slice(text), "日");
}

#[test]
fn zero_limit_yields_one_character_and_offsets_past_the_end_are_empty() {
    let text = "héllo";
    let range = Utf8Range::requested(text, Some(1), Some(0));
    assert_eq!((range.start, range.end), (1, 3), "progress is guaranteed");
    let end = Utf8Range::requested(text, Some(text.len()), Some(5));
    assert!(end.is_empty());
    assert_eq!((end.start, end.end), (text.len(), text.len()));
    let beyond = Utf8Range::requested(text, Some(999), None);
    assert!(beyond.is_empty());
    assert_eq!(beyond.start, text.len());
    let empty = Utf8Range::requested("", Some(3), Some(3));
    assert!(empty.is_empty());
}

#[test]
fn halving_moves_the_end_to_a_boundary_and_stops_at_one_character() {
    let text = "日本語です"; // 5 × 3 bytes
    let mut range = Utf8Range::requested(text, None, None);
    assert!(range.halve(text));
    assert_eq!(range.end, 6, "midpoint 7 moves back to the boundary at 6");
    assert!(range.halve(text));
    assert_eq!(range.end, 3);
    assert!(!range.halve(text), "one character left: cannot shrink");
    assert_eq!((range.start, range.end), (0, 3));
}

#[test]
fn errors_display_the_missing_reference() {
    let err = RecoveryError::MessageNotFound(MessageId::from("m-1"));
    assert_eq!(err.to_string(), "message not found: m-1");
    assert_eq!(err.message_id().as_str(), "m-1");
    let err = RecoveryError::ToolCallNotFound {
        message_id: MessageId::from("m-2"),
        tool_call_id: ToolCallId::from("call-9"),
    };
    assert_eq!(err.to_string(), "tool call call-9 not found in message m-2");
    assert_eq!(err.message_id().as_str(), "m-2");
}
