use super::*;
use crate::domain::message::ToolCall;

#[test]
fn clear_thinking_page_removes_all_thinking_metadata() {
    let mut value = serde_json::json!({
        "content": "answer",
        "thinking": "private",
        "thinkingOffset": 3,
        "thinkingLength": 9,
        "hasMoreThinking": true,
        "nextThinkingOffset": 6
    });
    clear_thinking_page(&mut value);
    assert_eq!(value, serde_json::json!({"content": "answer"}));
}

#[test]
fn message_to_json_range_returns_full_message_without_range_args() {
    let msg = Message::assistant("hello", vec![]);
    let json = message_to_json_range_for_response(&msg, None, None, None, None);

    assert_eq!(json["id"], msg.id().to_string());
    assert_eq!(json["content"], "hello");
    assert!(json.get("offset").is_none());
}

#[test]
fn message_to_json_range_clamps_to_utf8_boundaries() {
    let msg = Message::user("aé日z");
    let json = message_to_json_range_for_response(&msg, Some(2), None, Some(4), Some("req"));

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

/// #2226: `get_message` carries the message's turn origin, as `get_messages`
/// does; an unstamped message carries none.
#[test]
fn message_to_json_range_carries_the_turn_origin() {
    let mut msg = Message::assistant("REPORT", vec![]);
    let json = message_to_json_range_for_response(&msg, None, None, None, None);
    assert!(json["turnOrigin"].is_null());
    msg.turn_origin = crate::domain::turn_origin::TurnOrigin::ProgressNudge;
    let json = message_to_json_range_for_response(&msg, Some(0), None, Some(2), None);
    assert_eq!(json["turnOrigin"], "progressNudge");
}

/// #2404 (W2 review L4): `get_message` names a user message's kind, so a
/// reader tells a watermark cut's archive stub from a prompt the user
/// sent; an unmarked message names none.
#[test]
fn message_to_json_range_carries_the_user_kind() {
    let kind = |msg: &Message| message_to_json_range_for_response(msg, None, None, None, None);
    let prompt = crate::domain::turn_origin::prompt("do the thing".into());
    assert_eq!(kind(&prompt)["userKind"], "prompt");
    let stub = crate::domain::conversation::watermark_cut::archive_stub(4, Some("archive"));
    assert_eq!(kind(&stub)["userKind"], "archiveStub");
    let json = kind(&Message::user("feedback"));
    assert!(json["userKind"].is_null(), "{json}");
    let json = message_to_json_range_for_response(&stub, Some(0), None, Some(2), None);
    assert_eq!(json["userKind"], "archiveStub", "a ranged read too");
}
