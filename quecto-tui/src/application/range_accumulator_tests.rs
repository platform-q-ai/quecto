//! Direct pins for `RangeAccumulator` (#1221 parity contract, "Range assembly").
//!
//! Both callers (stub recall in `app_paged_history`, ref recovery in
//! `app_message_recovery`) map every `Err(_)` onto a terminal outcome — mark the
//! stub failed, or abandon the whole batch — so a misclassified error silently
//! drops user content. These tests pin each variant and each documented default
//! so the extraction cannot alter the classification.

use super::{RangeAccumulator, RangeError, RangeUpdate};
use serde_json::json;

fn acc(content: &str, offset: usize) -> RangeAccumulator {
    RangeAccumulator::new(content.to_string(), offset)
}

#[test]
fn missing_content_is_an_error() {
    assert_eq!(
        acc("", 0).apply(&json!({ "contentLength": 0 })),
        Err(RangeError::MissingContent),
        "a page without a content field must not be treated as an empty page"
    );
}

#[test]
fn a_response_offset_disagreeing_with_the_accumulator_is_rejected() {
    assert_eq!(
        acc("abc", 3).apply(&json!({ "content": "def", "offset": 9 })),
        Err(RangeError::OffsetMismatch),
        "a page starting at the wrong offset must never be appended"
    );
}

#[test]
fn an_absent_offset_defaults_to_zero() {
    assert_eq!(
        acc("", 0).apply(&json!({ "content": "hello" })),
        Ok(RangeUpdate::Complete("hello".into())),
        "an absent offset must default to 0 and match a fresh accumulator"
    );
    assert_eq!(
        acc("abc", 3).apply(&json!({ "content": "def" })),
        Err(RangeError::OffsetMismatch),
        "an absent offset must default to 0, not to the accumulator's offset"
    );
}

#[test]
fn an_absent_content_length_defaults_to_the_accumulated_length() {
    assert_eq!(
        acc("abc", 3).apply(&json!({ "content": "def", "offset": 3 })),
        Ok(RangeUpdate::Complete("abcdef".into())),
        "an absent contentLength must default to the accumulated length and complete"
    );
}

#[test]
fn more_content_without_a_next_offset_is_an_error() {
    assert_eq!(
        acc("", 0).apply(&json!({
            "content": "abc",
            "offset": 0,
            "contentLength": 10,
            "hasMoreContent": true,
        })),
        Err(RangeError::MissingNextOffset),
        "a continuation page must carry the offset to resume from"
    );
}

#[test]
fn a_non_progressing_next_offset_is_invalid_progress() {
    assert_eq!(
        acc("", 0).apply(&json!({
            "content": "abc",
            "offset": 0,
            "contentLength": 10,
            "hasMoreContent": true,
            "nextOffset": 0,
        })),
        Err(RangeError::InvalidProgress),
        "a next offset that does not advance would loop forever"
    );
}

#[test]
fn a_next_offset_beyond_the_content_length_is_invalid_progress() {
    assert_eq!(
        acc("", 0).apply(&json!({
            "content": "abc",
            "offset": 0,
            "contentLength": 10,
            "hasMoreContent": true,
            "nextOffset": 11,
        })),
        Err(RangeError::InvalidProgress),
        "a next offset past the advertised end must be rejected"
    );
}

#[test]
fn accumulating_past_the_advertised_length_is_invalid_progress() {
    assert_eq!(
        acc("already too long", 0).apply(&json!({
            "content": "more",
            "offset": 0,
            "contentLength": 5,
            "hasMoreContent": true,
            "nextOffset": 4,
        })),
        Err(RangeError::InvalidProgress),
        "overshooting the advertised length must be rejected"
    );
}

#[test]
fn a_final_page_shorter_than_advertised_is_a_length_mismatch() {
    assert_eq!(
        acc("", 0).apply(&json!({
            "content": "abc",
            "offset": 0,
            "contentLength": 10,
        })),
        Err(RangeError::LengthMismatch),
        "a truncated final page must not be delivered as complete content"
    );
}

#[test]
fn a_valid_continuation_returns_the_accumulated_prefix_and_next_offset() {
    assert_eq!(
        acc("abc", 3).apply(&json!({
            "content": "def",
            "offset": 3,
            "contentLength": 9,
            "hasMoreContent": true,
            "nextOffset": 6,
        })),
        Ok(RangeUpdate::Continue {
            content: "abcdef".into(),
            next_offset: 6,
        }),
        "a valid continuation must carry the accumulated prefix forward"
    );
}

#[test]
fn a_multi_page_body_reassembles_in_order() {
    let mut content = String::new();
    let mut offset = 0usize;
    let pages = ["one-", "two-", "three"];
    let total: usize = pages.iter().map(|p| p.len()).sum();

    for (i, page) in pages.iter().enumerate() {
        let last = i == pages.len() - 1;
        let next_offset = offset + page.len();
        let update = acc(&content, offset)
            .apply(&json!({
                "content": page,
                "offset": offset,
                "contentLength": total,
                "hasMoreContent": !last,
                "nextOffset": next_offset,
            }))
            .expect("each page must apply cleanly");
        match update {
            RangeUpdate::Continue {
                content: acc_content,
                next_offset: next,
            } => {
                assert!(!last, "only non-final pages may continue");
                content = acc_content;
                offset = next;
            }
            RangeUpdate::Complete(full) => {
                assert!(last, "only the final page may complete");
                content = full;
            }
        }
    }

    assert_eq!(
        content, "one-two-three",
        "a multi-page body must reassemble exactly and in order"
    );
}

/// `LengthMismatch` was only ever pinned SHORT (#1236 review). An over-long
/// final page must be rejected too: relaxing the check to `>=` shipped a body
/// carrying more bytes than the server advertised, and both callers treat an
/// unflagged body as trustworthy user content.
#[test]
fn a_final_page_longer_than_advertised_is_a_length_mismatch() {
    assert_eq!(
        acc("", 0).apply(&json!({
            "content": "abcdefghijkl",
            "offset": 0,
            "contentLength": 5,
        })),
        Err(RangeError::LengthMismatch),
        "an over-long body must not be delivered as complete content"
    );
}

#[test]
fn a_non_string_content_field_is_missing_content() {
    assert_eq!(
        acc("", 0).apply(&json!({ "content": 42, "contentLength": 2 })),
        Err(RangeError::MissingContent),
        "a non-string content field must be rejected, not coerced"
    );
}

/// A continuation that claims to resume exactly AT the advertised end has no
/// bytes left to deliver, so it is rejected rather than accepted as a page that
/// would loop forever waiting for content that cannot exist.
#[test]
fn a_next_offset_exactly_at_the_advertised_end_is_invalid_progress() {
    assert_eq!(
        acc("abc", 0).apply(&json!({
            "content": "abc",
            "offset": 0,
            "contentLength": 3,
            "hasMoreContent": true,
            "nextOffset": 3,
        })),
        Err(RangeError::InvalidProgress),
        "a continuation resuming at the advertised end can deliver nothing"
    );
}
