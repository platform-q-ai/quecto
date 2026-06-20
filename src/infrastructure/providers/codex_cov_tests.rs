//! Additional region-coverage unit tests for the Codex provider.
//!
//! Focus: pure logic only — streaming SSE handler (`CodexSseHandler`),
//! accumulator edge cases, request-body construction, and parsing error
//! paths. No live network: the handler is driven directly via an in-memory
//! mpsc channel.

use super::*;
use crate::domain::tool::ToolDefinition;

fn req<'a>(
    messages: &'a [Message],
    tools: &'a [ToolDefinition],
    model: &'a str,
    session_id: Option<&'a str>,
) -> ChatRequest<'a> {
    ChatRequest {
        messages,
        tools,
        model,
        max_tokens: 1024,
        temperature: 0.7,
        session_id,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    }
}

// --- CodexSseHandler streaming (process_line / on_eof) ---

#[tokio::test]
async fn handler_emits_text_delta_then_done_on_completed() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let mut handler = CodexSseHandler::new();

    let out = handler
        .process_line(
            r#"data: {"type":"response.output_text.delta","delta":"Hi"}"#,
            &tx,
        )
        .await;
    assert!(matches!(out, SseLineOutcome::Continue));
    match rx.recv().await.unwrap() {
        StreamEvent::TextDelta(t) => assert_eq!(t, "Hi"),
        other => panic!("unexpected: {other:?}"),
    }

    let out = handler
        .process_line(
            r#"data: {"type":"response.completed","response":{"usage":{"input_tokens":3,"output_tokens":1}}}"#,
            &tx,
        )
        .await;
    assert!(matches!(out, SseLineOutcome::Done));
    match rx.recv().await.unwrap() {
        StreamEvent::Done(resp) => {
            assert_eq!(resp.content.as_deref(), Some("Hi"));
            assert_eq!(resp.usage.unwrap().prompt_tokens, 3);
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn handler_done_marker_sends_done_event() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let mut handler = CodexSseHandler::new();

    let out = handler.process_line("data: [DONE]", &tx).await;
    assert!(matches!(out, SseLineOutcome::Done));
    assert!(matches!(rx.recv().await.unwrap(), StreamEvent::Done(_)));
}

#[tokio::test]
async fn handler_ignores_non_data_and_malformed_then_eof_done() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let mut handler = CodexSseHandler::new();

    assert!(matches!(
        handler.process_line(": keepalive", &tx).await,
        SseLineOutcome::Continue
    ));
    assert!(matches!(
        handler.process_line("data: {not json", &tx).await,
        SseLineOutcome::Continue
    ));
    assert!(rx.try_recv().is_err());

    handler.on_eof(&tx).await;
    match rx.recv().await.unwrap() {
        StreamEvent::Done(resp) => assert!(resp.content.is_none()),
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn handler_accumulates_tool_call_arguments() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let mut handler = CodexSseHandler::new();

    handler
        .process_line(
            r#"data: {"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","call_id":"c1","name":"bash","arguments":""}}"#,
            &tx,
        )
        .await;
    handler
        .process_line(
            r#"data: {"type":"response.function_call_arguments.delta","output_index":0,"delta":"{\"x\":1}"}"#,
            &tx,
        )
        .await;
    handler.on_eof(&tx).await;

    match rx.recv().await.unwrap() {
        StreamEvent::Done(resp) => {
            assert_eq!(resp.tool_calls.len(), 1);
            assert_eq!(resp.tool_calls[0].name, "bash");
            assert_eq!(resp.tool_calls[0].arguments, r#"{"x":1}"#);
        }
        other => panic!("unexpected: {other:?}"),
    }
}

// --- SseAccumulator edge cases ---

#[test]
fn accumulator_delta_for_unknown_output_index_is_ignored() {
    let mut acc = SseAccumulator::default();
    // No output_item.added registered this index, so the delta must be dropped.
    acc.handle_event(&serde_json::json!({
        "type": "response.function_call_arguments.delta",
        "output_index": 9,
        "delta": "ignored",
    }));
    let resp = acc.into_response();
    assert!(resp.tool_calls.is_empty());
    assert!(resp.content.is_none());
}

#[test]
fn accumulator_completed_without_response_field_leaves_usage_none() {
    let mut acc = SseAccumulator::default();
    acc.handle_event(&serde_json::json!({ "type": "response.completed" }));
    assert!(acc.into_response().usage.is_none());
}

#[test]
fn accumulator_item_added_non_function_call_is_skipped() {
    let mut acc = SseAccumulator::default();
    acc.handle_event(&serde_json::json!({
        "type": "response.output_item.added",
        "output_index": 0,
        "item": { "type": "reasoning" },
    }));
    assert!(acc.into_response().tool_calls.is_empty());
}

#[test]
fn accumulator_unknown_event_type_is_ignored() {
    let mut acc = SseAccumulator::default();
    acc.handle_event(&serde_json::json!({ "type": "response.created" }));
    let resp = acc.into_response();
    assert!(resp.content.is_none() && resp.tool_calls.is_empty());
}

// --- build_input edge cases ---

#[test]
fn build_input_concatenates_multiple_system_messages() {
    let messages = vec![
        Message::system("First."),
        Message::system("Second."),
        Message::user("Hi"),
    ];
    let (instructions, input) = CodexProvider::build_input(&messages);
    assert_eq!(instructions.unwrap(), "First.\nSecond.");
    assert_eq!(input.len(), 1);
    assert_eq!(input[0]["role"], "user");
}

#[test]
fn build_input_all_orphan_with_empty_content_emits_nothing() {
    // Assistant has only an orphaned tool call and no text → no item emitted.
    let mut assistant = Message::assistant("", vec![]);
    assistant.tool_calls = vec![ToolCall {
        id: "orphan".into(),
        name: "bash".into(),
        arguments: "{}".into(),
    }];
    let messages = vec![Message::user("go"), assistant];
    let (_inst, input) = CodexProvider::build_input(&messages);
    // Only the user message survives.
    assert_eq!(input.len(), 1);
    assert_eq!(input[0]["role"], "user");
}

// --- build_request_body with tools + session_id ---

#[test]
fn build_request_body_includes_tools_array_and_cache_key() {
    let messages = vec![Message::system("Be concise."), Message::user("Hi")];
    let tools = vec![ToolDefinition {
        name: "read".into(),
        description: "Read a file".into(),
        parameters_schema: r#"{"type":"object"}"#.into(),
    }];
    let body =
        CodexProvider::build_request_body(&req(&messages, &tools, "gpt-5.2", Some("uds:agent-1")));
    let tool_arr = body["tools"].as_array().expect("tools present");
    assert_eq!(tool_arr.len(), 1);
    assert_eq!(tool_arr[0]["name"], "read");
    let key = body["prompt_cache_key"].as_str().unwrap();
    assert!(key.starts_with("uds:"));
    assert!(!key.contains("agent-1"));
}

// --- parse_response error path ---

#[test]
fn parse_response_missing_output_is_error() {
    let body = serde_json::json!({ "usage": { "input_tokens": 1, "output_tokens": 1 } });
    let err = CodexProvider::parse_response(&body).unwrap_err();
    assert!(err.to_string().contains("missing output"));
}

#[test]
fn parse_response_skips_reasoning_items() {
    let body = serde_json::json!({
        "output": [
            { "type": "reasoning", "summary": "thinking" },
            { "type": "message", "content": [{ "type": "output_text", "text": "Done" }] },
        ]
    });
    let resp = CodexProvider::parse_response(&body).unwrap();
    assert_eq!(resp.content.unwrap(), "Done");
    assert!(resp.usage.is_none());
}

// --- name() ---

#[test]
fn provider_name_is_codex() {
    let p = CodexProvider::new("k".into(), "acct".into(), None);
    assert_eq!(p.name(), "codex");
}
