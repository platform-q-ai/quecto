use super::*;
use crate::domain::conversation::value_objects::message::ToolCall;

fn reply(stop_reason: Option<StopReason>, calls: &[(&str, &str)]) -> LlmResponse {
    LlmResponse {
        content: None,
        tool_calls: calls
            .iter()
            .map(|(name, arguments)| ToolCall {
                id: format!("call_{name}"),
                name: (*name).into(),
                arguments: (*arguments).into(),
            })
            .collect(),
        usage: None,
        stop_reason,
        thinking_blocks: vec![],
    }
}

const CUT: &str = r#"{"path":"big.txt","content":"li"#;

#[test]
fn a_call_with_cut_arguments_at_the_output_limit_is_cut_off() {
    let response = reply(Some(StopReason::MaxTokens), &[("write", CUT)]);
    assert_eq!(
        cut_off_tool_call(&response),
        Some(CutOffCall {
            id: "call_write",
            tool: "write"
        })
    );
}

#[test]
fn the_cut_off_call_is_found_behind_whole_ones() {
    let response = reply(
        Some(StopReason::MaxTokens),
        &[("read", r#"{"path":"a"}"#), ("write", CUT)],
    );
    assert_eq!(
        cut_off_tool_call(&response).map(|cut| cut.tool),
        Some("write")
    );
}

#[test]
fn whole_arguments_at_the_output_limit_are_no_cut_off() {
    let response = reply(Some(StopReason::MaxTokens), &[("write", r#"{"path":"a"}"#)]);
    assert_eq!(cut_off_tool_call(&response), None);
}

/// Review of #2598: the limit can stop a reply before its last call got
/// any argument text; that call was cut off, not called with `{}`.
#[test]
fn the_last_call_with_no_arguments_at_the_output_limit_is_cut_off() {
    let response = reply(
        Some(StopReason::MaxTokens),
        &[("read", r#"{"path":"a"}"#), ("write", "")],
    );
    assert_eq!(
        cut_off_tool_call(&response),
        Some(CutOffCall {
            id: "call_write",
            tool: "write"
        })
    );
}

#[test]
fn an_earlier_call_with_no_arguments_was_not_cut_off() {
    let response = reply(
        Some(StopReason::MaxTokens),
        &[("ping", ""), ("read", r#"{"path":"a"}"#)],
    );
    assert_eq!(cut_off_tool_call(&response), None);
    let ended = reply(Some(StopReason::ToolUse), &[("ping", "")]);
    assert_eq!(cut_off_tool_call(&ended), None, "no limit, no cut-off");
}

#[test]
fn invalid_arguments_without_the_output_limit_are_no_cut_off() {
    for stop in [
        None,
        Some(StopReason::EndTurn),
        Some(StopReason::ToolUse),
        Some(StopReason::Unknown("length".into())),
    ] {
        let response = reply(stop.clone(), &[("write", CUT)]);
        assert_eq!(cut_off_tool_call(&response), None, "{stop:?}");
    }
}

#[test]
fn a_reply_with_no_call_cut_nothing_off() {
    assert_eq!(
        cut_off_tool_call(&reply(Some(StopReason::MaxTokens), &[])),
        None
    );
}

#[test]
fn only_allowlisted_responses_reasons_are_the_output_limit() {
    assert!(is_responses_output_limit("max_output_tokens"));
    for reason in ["content_filter", "", "MAX_OUTPUT_TOKENS", "max_tokens"] {
        assert!(!is_responses_output_limit(reason), "{reason:?}");
    }
}
