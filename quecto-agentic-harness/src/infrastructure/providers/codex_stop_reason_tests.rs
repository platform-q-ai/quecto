//! #2157: a Codex response that ends in function calls stops for tool use.
use super::*;
use crate::domain::message::StopReason;

#[test]
fn a_completed_response_with_function_calls_stops_for_tool_use() {
    let sse = r#"data: {"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","call_id":"c1","name":"read","arguments":""}}
data: {"type":"response.function_call_arguments.done","output_index":0,"arguments":"{\"path\":\"a.rs\"}"}
data: {"type":"response.completed","response":{"status":"completed"}}
"#;
    let resp = CodexProvider::parse_sse_response(sse).unwrap();
    assert_eq!(resp.tool_calls.len(), 1);
    assert_eq!(resp.stop_reason, Some(StopReason::ToolUse));
}

#[test]
fn a_completed_response_with_text_alone_ends_its_turn() {
    let sse = r#"data: {"type":"response.output_text.delta","delta":"done"}
data: {"type":"response.completed","response":{"status":"completed"}}
"#;
    let resp = CodexProvider::parse_sse_response(sse).unwrap();
    assert_eq!(resp.stop_reason, Some(StopReason::EndTurn));
}

#[test]
fn an_incomplete_response_with_calls_keeps_its_reason() {
    let sse = r#"data: {"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","call_id":"c1","name":"read","arguments":"{}"}}
data: {"type":"response.completed","response":{"status":"incomplete"}}
"#;
    let resp = CodexProvider::parse_sse_response(sse).unwrap();
    assert_eq!(resp.stop_reason, Some(StopReason::MaxTokens));
}
