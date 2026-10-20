use super::*;

#[test]
fn parse_get_message_accepts_valid_busy_lookup_fields() {
    let parsed = parse(
        r#"{"type":"get_message","id":"req","messageId":"m1","toolCallId":"tc","offset":2,"limit":5}"#,
    )
    .expect("busy get_message parses");

    assert_eq!(parsed.request_id.as_deref(), Some("req"));
    assert_eq!(parsed.message_id, "m1");
    assert_eq!(parsed.tool_call_id.as_deref(), Some("tc"));
    assert_eq!(parsed.offset, Some(2));
    assert_eq!(parsed.limit, Some(5));
}

#[test]
fn parse_get_message_rejects_non_lookup_child_forward_and_missing_id() {
    assert_eq!(parse("not-json"), None);
    assert_eq!(parse(r#"{"type":"get_state","messageId":"m1"}"#), None);
    assert_eq!(
        parse(r#"{"type":"get_message","agent_id":"child","messageId":"m1"}"#),
        None
    );
    assert_eq!(parse(r#"{"type":"get_message"}"#), None);
}

#[test]
fn parse_get_message_ignores_non_numeric_ranges() {
    let parsed = parse(r#"{"type":"get_message","messageId":"m1","offset":"x","limit":-1}"#)
        .expect("message id is sufficient");

    assert_eq!(parsed.offset, None);
    assert_eq!(parsed.limit, None);
}

#[test]
fn issue_1060_busy_reader_parses_only_un_targeted_get_message() {
    // Un-targeted lookup: the busy reader services it from the master snapshot.
    assert_eq!(
        parse(r#"{"type":"get_message","id":"r1","messageId":"m1"}"#),
        Some(ParsedGetMessage {
            request_id: Some("r1".into()),
            message_id: "m1".into(),
            tool_call_id: None,
            offset: None,
            thinking_offset: None,
            limit: None
        })
    );
    // An agent-targeted lookup MUST fall through (None) so dispatch forwards it
    // to the child. The wire key is snake_case `agent_id` (Command::GetMessage
    // has no rename) — asserting against the real key guards the #1060 casing
    // bug where the master wrongly served child ids from its own snapshot.
    assert_eq!(
        parse(r#"{"type":"get_message","id":"r1","messageId":"m1","agent_id":"child"}"#),
        None
    );
}
