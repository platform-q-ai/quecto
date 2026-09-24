//! #2123: tool-call arguments are replayed as an object and never lost.
use super::*;
use crate::domain::message::ToolCall;

#[test]
fn build_input_sends_an_object_for_a_call_with_invalid_arguments() {
    // #2123: arguments truncated at the output limit must not be replayed.
    let mut assistant_msg = Message::assistant("", vec![]);
    assistant_msg.tool_calls = vec![ToolCall {
        id: "call_1".to_string(),
        name: "bash".into(),
        arguments: r#"{"command":"cat /very/lo"#.to_string(),
    }];
    let messages = vec![
        Message::user("go"),
        assistant_msg,
        Message::tool("call_1", "invalid"),
    ];
    let (_, input) = CodexProvider::build_input(&messages);
    assert_eq!(input[1]["type"], "function_call");
    assert_eq!(input[1]["arguments"], "{}");
}

#[test]
fn parse_sse_takes_the_complete_arguments_from_the_done_event() {
    // #2123: arguments that arrive only in `.done` (no deltas) must not be
    // lost, or the call would run with `{}` instead of what the model sent.
    let sse = r#"data: {"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","call_id":"c1","name":"read","arguments":""}}
data: {"type":"response.function_call_arguments.done","output_index":0,"arguments":"{\"path\":\"a.rs\"}"}
data: [DONE]
"#;
    let resp = CodexProvider::parse_sse_response(sse).unwrap();
    assert_eq!(resp.tool_calls[0].arguments, r#"{"path":"a.rs"}"#);
}

#[test]
fn parse_response_keeps_arguments_sent_as_an_object() {
    let body = serde_json::json!({"output": [
        {"type": "function_call", "call_id": "c1", "name": "read", "arguments": {"path": "a.rs"}}
    ]});
    let resp = CodexProvider::parse_response(&body).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&resp.tool_calls[0].arguments).unwrap();
    assert_eq!(parsed, serde_json::json!({"path": "a.rs"}));
}

#[test]
fn parse_sse_the_done_arguments_win_over_differing_deltas() {
    let sse = r#"data: {"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","call_id":"c1","name":"read","arguments":""}}
data: {"type":"response.function_call_arguments.delta","output_index":0,"delta":"{\"path\":\"a"}
data: {"type":"response.function_call_arguments.done","output_index":0,"arguments":"{\"path\":\"a.rs\"}"}
data: [DONE]
"#;
    let resp = CodexProvider::parse_sse_response(sse).unwrap();
    assert_eq!(resp.tool_calls[0].arguments, r#"{"path":"a.rs"}"#);
}
