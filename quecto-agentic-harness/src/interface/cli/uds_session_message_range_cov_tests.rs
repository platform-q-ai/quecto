use super::*;
use crate::domain::message::ToolCall;

#[test]
fn message_to_json_range_returns_full_message_without_range_args() {
    let msg = Message::assistant("hello", vec![]);
    let json = message_to_json_range(&msg, None, None);

    assert_eq!(json["id"], msg.id().to_string());
    assert_eq!(json["content"], "hello");
    assert!(json.get("offset").is_none());
}

#[test]
fn message_to_json_range_clamps_to_utf8_boundaries() {
    let msg = Message::user("aé日z");
    let json = message_to_json_range_for_response(&msg, Some(2), Some(4), Some("req"));

    assert_eq!(json["offset"], 1);
    assert_eq!(json["content"], "é");
    assert_eq!(json["nextOffset"], 3);
    assert_eq!(json["contentLength"], "aé日z".len());
    assert_eq!(json["hasMoreContent"], true);
}

#[test]
fn tool_call_arguments_range_returns_slice_metadata_and_none_for_missing_call() {
    let msg = Message::assistant(
        "",
        vec![ToolCall {
            id: "tc1".into(),
            name: "bash".into(),
            arguments: "aé日z".into(),
        }],
    );

    let json = tool_call_arguments_to_json_range_for_response(&msg, "tc1", Some(2), Some(4), None)
        .expect("tool call range exists");
    assert_eq!(json["toolCallId"], "tc1");
    assert_eq!(json["toolName"], "bash");
    assert_eq!(json["arguments"], "é");
    assert_eq!(json["offset"], 1);
    assert_eq!(json["nextOffset"], 3);
    assert_eq!(json["argumentsLength"], "aé日z".len());
    assert_eq!(json["hasMoreArguments"], true);

    assert!(
        tool_call_arguments_to_json_range_for_response(&msg, "missing", None, None, None).is_none()
    );
}
