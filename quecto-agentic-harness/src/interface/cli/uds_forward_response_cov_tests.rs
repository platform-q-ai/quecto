use super::*;

#[test]
fn parse_forwarded_get_messages_returns_data_for_success() {
    let data = parse_forwarded_get_messages(
        r#"{"success":true,"command":"get_messages","data":{"messages":[{"id":"m1"}]}}"#,
    )
    .expect("success response parses");

    assert_eq!(data["messages"][0]["id"], "m1");
}

#[test]
fn parse_forwarded_get_messages_reports_failure_error_or_default() {
    assert_eq!(
        parse_forwarded_get_messages(
            r#"{"success":false,"command":"get_messages","error":"child busy"}"#,
        )
        .unwrap_err(),
        "child busy"
    );
    assert_eq!(
        parse_forwarded_get_messages(r#"{"success":false,"command":"get_messages"}"#).unwrap_err(),
        "get_messages failed"
    );
}

#[test]
fn parse_forwarded_get_messages_rejects_wrong_command_missing_data_and_bad_json() {
    assert_eq!(
        parse_forwarded_get_messages(r#"{"success":true,"command":"get_state","data":{}}"#)
            .unwrap_err(),
        "unexpected child response command"
    );
    assert_eq!(
        parse_forwarded_get_messages(r#"{"success":true,"command":"get_messages"}"#).unwrap_err(),
        "get_messages response missing data"
    );
    assert!(
        parse_forwarded_get_messages("not json")
            .unwrap_err()
            .contains("expected")
    );
}
