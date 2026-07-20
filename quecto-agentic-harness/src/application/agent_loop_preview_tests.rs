use super::*;

#[test]
fn empty_content_previews_as_empty() {
    assert_eq!(tool_result_preview(""), "");
}

#[test]
fn multibyte_content_ending_exactly_on_cap_boundary_is_not_truncated_further() {
    // '€' is 3 bytes; DEFAULT_OUTPUT_CAP_BYTES is a multiple of 1024, not
    // of 3, so pad with ASCII to land a char boundary exactly at the cap,
    // then overflow by one more char so truncation is required.
    let euros = DEFAULT_OUTPUT_CAP_BYTES / 3;
    let padding = DEFAULT_OUTPUT_CAP_BYTES - euros * 3;
    let mut content = "a".repeat(padding);
    content.push_str(&"€".repeat(euros + 1));
    let preview = tool_result_preview(&content);
    assert_eq!(
        preview.len(),
        DEFAULT_OUTPUT_CAP_BYTES,
        "cap lands on a char boundary, so no extra backoff is allowed"
    );
    assert_eq!(preview, content[..DEFAULT_OUTPUT_CAP_BYTES]);
}

#[test]
fn mixed_ascii_and_multibyte_straddling_the_cap_truncates_to_char_boundary() {
    // Place a 3-byte char straddling the cap: bytes cap-1..cap+2.
    let mut content = "a".repeat(DEFAULT_OUTPUT_CAP_BYTES - 1);
    content.push_str("€€");
    let preview = tool_result_preview(&content);
    assert_eq!(
        preview.len(),
        DEFAULT_OUTPUT_CAP_BYTES - 1,
        "must back off to the last char boundary before the cap"
    );
    assert!(preview.chars().all(|ch| ch == 'a'));
}
