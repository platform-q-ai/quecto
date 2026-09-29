//! A denial's input preview is cut at exactly its byte budget, backed off
//! to a character boundary (#2285).

use serde_json::json;

use super::*;

/// A JSON string value serialises as `"` + text + `"`: the text starts at
/// byte 1.
fn preview_of(text: &str) -> String {
    input_preview(&json!(text))
}

#[test]
fn the_preview_budget_is_512_bytes() {
    assert_eq!(AUDIT_INPUT_PREVIEW_BYTES, 512);
}

#[test]
fn an_input_of_exactly_the_budget_is_kept_whole() {
    let text = "x".repeat(AUDIT_INPUT_PREVIEW_BYTES - 2);
    let preview = preview_of(&text);
    assert_eq!(preview.len(), AUDIT_INPUT_PREVIEW_BYTES);
    assert_eq!(preview, format!("\"{text}\""));
}

#[test]
fn an_input_one_byte_over_the_budget_is_cut_at_the_budget() {
    let text = "x".repeat(AUDIT_INPUT_PREVIEW_BYTES - 1);
    let preview = preview_of(&text);
    assert_eq!(preview.len(), AUDIT_INPUT_PREVIEW_BYTES);
    assert_eq!(
        preview,
        format!("\"{}", "x".repeat(AUDIT_INPUT_PREVIEW_BYTES - 1))
    );
}

#[test]
fn a_two_byte_char_straddling_the_budget_is_dropped_whole() {
    // "é" at bytes 1-2, 3-4, …, 511-512: the budget splits the last one.
    let preview = preview_of(&"é".repeat(AUDIT_INPUT_PREVIEW_BYTES));
    assert_eq!(preview.len(), AUDIT_INPUT_PREVIEW_BYTES - 1);
    assert_eq!(preview, format!("\"{}", "é".repeat(255)));
}

#[test]
fn a_four_byte_char_straddling_the_budget_backs_off_three_bytes() {
    // "😀" at bytes 1-4, 5-8, …, 509-512: byte 512 is inside the last one.
    let preview = preview_of(&"😀".repeat(AUDIT_INPUT_PREVIEW_BYTES));
    assert_eq!(preview.len(), AUDIT_INPUT_PREVIEW_BYTES - 3);
    assert_eq!(preview, format!("\"{}", "😀".repeat(127)));
}
