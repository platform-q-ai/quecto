use super::{LONG_REQUEST_ID_REGRESSION_LEN, message_to_json_range_for_response};
use crate::domain::message::{Message, ToolCall};
use crate::interface::cli::protocol::AgentEvent;

/// #1103 review: range fitting must include the actual response id, not a fixed
/// envelope reserve. A long id should still produce a success frame under the
/// shared protocol cap by shrinking the returned content page.
#[test]
fn ranged_get_message_accounts_for_long_request_id() {
    let body = "x".repeat(crate::infrastructure::line_cap::EVENT_LINE_JSON_BUDGET);
    let msg = Message::assistant(body, vec![]);
    let request_id = "r".repeat(LONG_REQUEST_ID_REGRESSION_LEN);

    let data = message_to_json_range_for_response(&msg, Some(0), None, Some(&request_id));
    let line = AgentEvent::ok(Some(&request_id), "get_message", Some(data.clone())).to_json_line();

    assert!(
        line.len() <= crate::infrastructure::line_cap::EVENT_LINE_JSON_BUDGET,
        "ranged response exceeded frame budget with long request id: {} > {}",
        line.len(),
        crate::infrastructure::line_cap::EVENT_LINE_JSON_BUDGET
    );
    assert!(
        data["nextOffset"].as_u64().unwrap() > 0,
        "long request id should shrink the page, not return an empty page for simple content"
    );
    assert_eq!(data["hasMoreContent"].as_bool(), Some(true));
}

#[test]
fn tool_call_argument_range_reassembles_utf8_payload_with_bounded_frames() {
    let arguments = "λ".repeat(crate::infrastructure::line_cap::EVENT_LINE_JSON_BUDGET);
    let msg = Message::assistant(
        "small content",
        vec![ToolCall {
            id: "call-large".into(),
            name: "large_tool".into(),
            arguments: arguments.clone(),
        }],
    );

    let mut offset = 0usize;
    let mut recovered = String::new();
    loop {
        let data = super::tool_call_arguments_to_json_range_for_response(
            &msg,
            "call-large",
            Some(offset),
            Some(64 * 1024),
            Some("recover-tool-call"),
        )
        .expect("tool call exists");
        let line = AgentEvent::ok(Some("recover-tool-call"), "get_message", Some(data.clone()))
            .to_json_line();
        assert!(
            line.len() <= crate::infrastructure::line_cap::EVENT_LINE_JSON_BUDGET,
            "tool-call argument page must fit the protocol frame"
        );
        recovered.push_str(data["arguments"].as_str().expect("arguments page"));
        assert_eq!(data["toolCallId"], "call-large");
        assert_eq!(data["argumentsLength"], arguments.len());
        if data["hasMoreArguments"] == false {
            break;
        }
        let next = data["nextOffset"].as_u64().expect("next offset") as usize;
        assert!(next > offset, "argument paging must make progress");
        offset = next;
    }

    assert_eq!(recovered, arguments);
}

#[test]
fn ranged_get_message_preserves_visible_thinking_for_recovery() {
    use crate::domain::message::ThinkingBlock;

    let mut msg = Message::assistant("answer", vec![]);
    msg.thinking_blocks.push(ThinkingBlock::Normal {
        thinking: "visible reasoning".into(),
        signature: "private".into(),
    });

    let data = message_to_json_range_for_response(&msg, Some(0), Some(1024), Some("recover"));

    assert_eq!(data["content"], "answer");
    assert_eq!(data["thinking"][0]["kind"], "text");
    assert_eq!(data["thinking"][0]["text"], "visible reasoning");
    assert!(!serde_json::to_string(&data).unwrap().contains("private"));
}

#[test]
fn ranged_get_message_with_huge_thinking_fits_protocol_frame() {
    use crate::domain::message::ThinkingBlock;

    let mut msg = Message::assistant("answer".repeat(1024), vec![]);
    msg.thinking_blocks.push(ThinkingBlock::Normal {
        thinking: "r".repeat(crate::infrastructure::line_cap::EVENT_LINE_JSON_BUDGET),
        signature: "private".into(),
    });

    let data = message_to_json_range_for_response(&msg, Some(0), None, Some("recover"));
    let line = AgentEvent::ok(Some("recover"), "get_message", Some(data)).to_json_line();

    assert!(
        line.len() <= crate::infrastructure::line_cap::EVENT_LINE_JSON_BUDGET,
        "ranged get_message must include thinking in its frame-budget fit: {} > {}",
        line.len(),
        crate::infrastructure::line_cap::EVENT_LINE_JSON_BUDGET
    );
}
