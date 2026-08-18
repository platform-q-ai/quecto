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

    let data = message_to_json_range_for_response(&msg, Some(0), None, None, Some(&request_id));
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

    let data = message_to_json_range_for_response(&msg, Some(0), None, Some(1024), Some("recover"));

    assert_eq!(data["content"], "answer");
    assert_eq!(data["thinking"][0]["kind"], "text");
    assert_eq!(data["thinking"][0]["text"], "visible reasoning");
    assert!(!serde_json::to_string(&data).unwrap().contains("private"));
}

#[test]
fn ranged_get_message_with_huge_thinking_returns_recoverable_thinking_page() {
    use crate::domain::message::ThinkingBlock;

    let visible = "r".repeat(crate::infrastructure::line_cap::EVENT_LINE_JSON_BUDGET);
    let mut msg = Message::assistant("answer".repeat(1024), vec![]);
    msg.thinking_blocks.push(ThinkingBlock::Normal {
        thinking: visible.clone(),
        signature: "private".into(),
    });

    let data = message_to_json_range_for_response(&msg, Some(0), None, Some(64), Some("recover"));
    let line = AgentEvent::ok(Some("recover"), "get_message", Some(data.clone())).to_json_line();

    assert!(
        line.len() <= crate::infrastructure::line_cap::EVENT_LINE_JSON_BUDGET,
        "thinking page must fit protocol frame"
    );
    let thinking = data["thinking"][0]["text"]
        .as_str()
        .expect("oversized visible thinking must be recoverable as a bounded page");
    assert!(!thinking.is_empty(), "thinking page should make progress");
    assert!(visible.starts_with(thinking));
    assert_eq!(data["thinkingOffset"], 0);
    assert!(data["nextThinkingOffset"].as_u64().unwrap() > 0);
    assert_eq!(data["thinkingLength"], visible.len());
    assert_eq!(data["hasMoreThinking"], true);
    assert!(!serde_json::to_string(&data).unwrap().contains("private"));
}

#[test]
fn ranged_get_message_reassembles_oversized_visible_thinking() {
    use crate::domain::message::ThinkingBlock;

    let visible = "λ".repeat(crate::infrastructure::line_cap::EVENT_LINE_JSON_BUDGET);
    let mut msg = Message::assistant("answer".repeat(1024), vec![]);
    msg.thinking_blocks.push(ThinkingBlock::Normal {
        thinking: visible.clone(),
        signature: "private".into(),
    });
    msg.thinking_blocks.push(ThinkingBlock::Redacted {
        data: "secret-redacted-blob".into(),
    });

    let mut offset = 0usize;
    let mut recovered = String::new();
    loop {
        let data = message_to_json_range_for_response(
            &msg,
            Some(0),
            Some(offset),
            Some(64),
            Some("recover"),
        );
        let line =
            AgentEvent::ok(Some("recover"), "get_message", Some(data.clone())).to_json_line();
        assert!(line.len() <= crate::infrastructure::line_cap::EVENT_LINE_JSON_BUDGET);
        assert_eq!(data["thinkingOffset"].as_u64(), Some(offset as u64));
        assert!(
            !serde_json::to_string(&data)
                .unwrap()
                .contains("secret-redacted-blob")
        );
        for block in data["thinking"].as_array().expect("thinking blocks") {
            match block["kind"].as_str() {
                Some("text") => recovered.push_str(block["text"].as_str().expect("thinking page")),
                Some("redacted") => recovered.push_str("[redacted]"),
                other => panic!("unexpected thinking kind: {other:?}"),
            }
        }
        if data["hasMoreThinking"] == false {
            break;
        }
        let next = data["nextThinkingOffset"].as_u64().unwrap() as usize;
        assert!(
            next > data["thinkingOffset"].as_u64().unwrap() as usize,
            "thinking paging must make progress"
        );
        offset = next;
    }

    assert_eq!(recovered, format!("{visible}[redacted]"));
    assert!(!recovered.contains("secret-redacted-blob"));
}

#[test]
fn ranged_get_message_pages_redacted_placeholder_boundaries() {
    use crate::domain::message::ThinkingBlock;

    let mut msg = Message::assistant("answer", vec![]);
    msg.thinking_blocks.push(ThinkingBlock::Normal {
        thinking: "λx".into(),
        signature: "private".into(),
    });
    msg.thinking_blocks.push(ThinkingBlock::Redacted {
        data: "secret-redacted-blob".into(),
    });
    msg.thinking_blocks.push(ThinkingBlock::Normal {
        thinking: "tail".into(),
        signature: "private2".into(),
    });

    let redacted = message_to_json_range_for_response(
        &msg,
        Some(0),
        Some("λx".len()),
        Some(1),
        Some("recover"),
    );
    assert_eq!(redacted["thinkingOffset"].as_u64(), Some("λx".len() as u64));
    assert!(redacted["nextThinkingOffset"].as_u64().unwrap() > "λx".len() as u64);
    assert_eq!(redacted["thinking"][0]["kind"], "redacted");
    assert!(
        !serde_json::to_string(&redacted)
            .unwrap()
            .contains("secret-redacted-blob")
    );

    let after = message_to_json_range_for_response(
        &msg,
        Some(0),
        Some("λx".len() + 1),
        Some(4),
        Some("recover"),
    );
    assert_eq!(after["thinking"][0]["kind"], "text");
    assert_eq!(after["thinking"][0]["text"], "tail");

    let beyond = message_to_json_range_for_response(
        &msg,
        Some(0),
        Some("λx".len() + 1 + "tail".len()),
        Some(4),
        Some("recover"),
    );
    assert!(beyond.get("thinking").is_none());
}

#[test]
fn ranged_get_message_with_huge_thinking_fits_protocol_frame() {
    use crate::domain::message::ThinkingBlock;

    let mut msg = Message::assistant("answer".repeat(1024), vec![]);
    msg.thinking_blocks.push(ThinkingBlock::Normal {
        thinking: "r".repeat(crate::infrastructure::line_cap::EVENT_LINE_JSON_BUDGET),
        signature: "private".into(),
    });

    let data = message_to_json_range_for_response(&msg, Some(0), None, None, Some("recover"));
    let line = AgentEvent::ok(Some("recover"), "get_message", Some(data)).to_json_line();

    assert!(
        line.len() <= crate::infrastructure::line_cap::EVENT_LINE_JSON_BUDGET,
        "ranged get_message must include thinking in its frame-budget fit: {} > {}",
        line.len(),
        crate::infrastructure::line_cap::EVENT_LINE_JSON_BUDGET
    );
}

#[test]
fn unranged_get_message_with_huge_thinking_falls_back_to_bounded_page() {
    use crate::domain::message::ThinkingBlock;

    let mut msg = Message::assistant("answer".repeat(1024), vec![]);
    msg.thinking_blocks.push(ThinkingBlock::Normal {
        thinking: "r".repeat(crate::infrastructure::line_cap::EVENT_LINE_JSON_BUDGET),
        signature: "private".into(),
    });

    let data = message_to_json_range_for_response(&msg, None, None, None, Some("recover"));
    let line = AgentEvent::ok(Some("recover"), "get_message", Some(data.clone())).to_json_line();

    assert!(line.len() <= crate::infrastructure::line_cap::EVENT_LINE_JSON_BUDGET);
    assert!(data["nextOffset"].as_u64().unwrap() > data["offset"].as_u64().unwrap());
}

#[test]
fn ranged_get_message_with_huge_thinking_still_makes_progress() {
    use crate::domain::message::ThinkingBlock;

    let mut msg = Message::assistant("abcdef", vec![]);
    msg.thinking_blocks.push(ThinkingBlock::Normal {
        thinking: "r".repeat(crate::infrastructure::line_cap::EVENT_LINE_JSON_BUDGET),
        signature: "private".into(),
    });

    let data = message_to_json_range_for_response(&msg, Some(0), None, Some(6), Some("recover"));

    assert!(data["nextOffset"].as_u64().unwrap() > 0);
    assert_eq!(data["hasMoreContent"].as_bool(), Some(false));
}
