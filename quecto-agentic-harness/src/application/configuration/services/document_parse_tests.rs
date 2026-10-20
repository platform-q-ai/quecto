use super::*;

#[test]
fn a_malformed_or_truncated_document_is_placed_never_quoted() {
    assert_eq!(
        parse_document(b"{ sk-FAKE-SECRET").unwrap_err(),
        "malformed JSON at line 1 column 3"
    );
    assert_eq!(
        parse_document(b"{\"a\": ").unwrap_err(),
        "the file ends early at line 1 column 6"
    );
    assert_eq!(parse_document(b"{}").unwrap(), serde_json::json!({}));
}
