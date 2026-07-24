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
async fn handler_ignores_non_data_and_malformed_then_eof_error() {
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
        StreamEvent::Error(e) => assert!(e.contains("ended without completion"), "{e}"),
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

// --- parse_response: multiple output_text parts accumulate (Some(c) arm) ---

#[test]
fn parse_response_concatenates_multiple_output_text_parts() {
    // A single message with two output_text parts exercises the `Some(c) =>
    // c.push_str(text)` accumulation arm (the single-part path hits `None`).
    let body = serde_json::json!({
        "output": [
            { "type": "message", "content": [
                { "type": "output_text", "text": "Hello " },
                { "type": "output_text", "text": "world" },
            ] }
        ]
    });
    let resp = CodexProvider::parse_response(&body).unwrap();
    assert_eq!(resp.content.unwrap(), "Hello world");
    assert!(resp.tool_calls.is_empty());
}

// --- parse_sse_response: non-`data:` lines are skipped ---

#[test]
fn parse_sse_response_skips_non_data_lines() {
    // event/comment/blank lines must hit the `continue` branch, then the
    // surviving data lines assemble normally.
    let sse = "event: ping\n\
               : keepalive comment\n\
               \n\
               data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hi\"}\n\
               data: [DONE]\n";
    let resp = CodexProvider::parse_sse_response(sse).unwrap();
    assert_eq!(resp.content.unwrap(), "Hi");
}

// --- public test-support accessors delegate to private builders ---

#[test]
fn public_accessors_delegate_to_private_builders() {
    let messages = vec![Message::system("Sys"), Message::user("U")];
    let tools: Vec<ToolDefinition> = vec![];
    let body = CodexProvider::build_request_body_public(&req(&messages, &tools, "gpt-5.2", None));
    assert_eq!(body["model"], "gpt-5.2");

    let (inst, input) = CodexProvider::build_input_public(&messages);
    assert_eq!(inst.unwrap(), "Sys");
    assert_eq!(input.len(), 1);

    let resp = CodexProvider::parse_sse_response_public(
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"x\"}\ndata: [DONE]\n",
    )
    .unwrap();
    assert_eq!(resp.content.unwrap(), "x");
}

// --- SseAccumulator: output_item.added with no "item" field is ignored ---

#[test]
fn accumulator_item_added_without_item_field_is_ignored() {
    let mut acc = SseAccumulator::default();
    acc.handle_event(&serde_json::json!({
        "type": "response.output_item.added",
        "output_index": 0,
    }));
    assert!(acc.into_response().tool_calls.is_empty());
}

// --- chat_stream delegates to chat (validation path, no network) ---

#[tokio::test]
async fn chat_stream_delegates_to_chat_validation() {
    let provider = CodexProvider::new("k".into(), "acct".into(), None);
    let messages = vec![Message::user("U")]; // no system message → missing instructions
    let err = provider
        .chat_stream(req(&messages, &[], "gpt-5.2", None))
        .await
        .expect_err("missing instructions must error");
    assert!(
        err.to_string().contains("requires instructions"),
        "got: {err}"
    );
}

// --- handler: output_text.delta event without a "delta" string ---

#[tokio::test]
async fn handler_output_text_delta_missing_field_emits_no_text() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let mut handler = CodexSseHandler::new();
    let out = handler
        .process_line(r#"data: {"type":"response.output_text.delta"}"#, &tx)
        .await;
    assert!(matches!(out, SseLineOutcome::Continue));
    assert!(rx.try_recv().is_err(), "no TextDelta should be emitted");
    handler.on_eof(&tx).await;
    match rx.recv().await.unwrap() {
        StreamEvent::Error(e) => assert!(e.contains("ended without completion"), "{e}"),
        other => panic!("unexpected: {other:?}"),
    }
}

// --- chat_stream_incremental: validation error surfaces over the channel ---

#[tokio::test]
async fn chat_stream_incremental_emits_error_on_invalid_model() {
    // Provider-qualified model fails validate_request before any spawn/network,
    // and the error is delivered via the returned channel.
    let provider = CodexProvider::new("k".into(), "acct".into(), None);
    let messages = vec![Message::system("Sys"), Message::user("U")];
    let mut rx = provider
        .chat_stream_incremental(req(&messages, &[], "openai/gpt-5.2", None))
        .await;
    match rx.recv().await.unwrap() {
        StreamEvent::Error(e) => assert!(e.contains("bare model id"), "got: {e}"),
        other => panic!("unexpected: {other:?}"),
    }
}

// --- chat_stream_incremental + pump_codex_sse (local mock server) ---
// These drive the streaming spawn path against a wiremock mock (localhost,
// deterministic — not a live API), the only way to exercise pump_codex_sse.

#[tokio::test]
async fn chat_stream_incremental_pumps_sse_to_done() {
    let server = wiremock::MockServer::start().await;
    let sse = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hi\"}\n\
               data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n";
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(sse))
        .mount(&server)
        .await;
    let provider = CodexProvider::new("k".into(), "acct".into(), Some(server.uri()));
    let messages = vec![Message::system("Sys"), Message::user("U")];
    let mut rx = provider
        .chat_stream_incremental(req(&messages, &[], "gpt-5.2", None))
        .await;

    let mut saw_done = false;
    while let Some(ev) = rx.recv().await {
        if let StreamEvent::Done(resp) = ev {
            assert_eq!(resp.content.as_deref(), Some("Hi"));
            saw_done = true;
        }
    }
    assert!(saw_done, "expected a Done event");
}

#[tokio::test]
async fn chat_stream_incremental_emits_http_error_status() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(wiremock::ResponseTemplate::new(401).set_body_string("nope"))
        .mount(&server)
        .await;
    let provider = CodexProvider::new("k".into(), "acct".into(), Some(server.uri()));
    let messages = vec![Message::system("Sys"), Message::user("U")];
    let mut rx = provider
        .chat_stream_incremental(req(&messages, &[], "gpt-5.2", None))
        .await;
    match rx.recv().await.unwrap() {
        StreamEvent::Error(e) => assert!(e.contains("HTTP 401 from Codex"), "got: {e}"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn chat_stream_incremental_emits_request_failed_on_unreachable() {
    // Port 1 is unbound → connection refused → the request-failed branch.
    let provider = CodexProvider::new("k".into(), "acct".into(), Some("http://127.0.0.1:1".into()));
    let messages = vec![Message::system("Sys"), Message::user("U")];
    let mut rx = provider
        .chat_stream_incremental(req(&messages, &[], "gpt-5.2", None))
        .await;
    match rx.recv().await.unwrap() {
        StreamEvent::Error(e) => assert!(e.contains("Codex request failed"), "got: {e}"),
        other => panic!("unexpected: {other:?}"),
    }
}

// --- name() ---

#[test]
fn provider_name_is_codex() {
    let p = CodexProvider::new("k".into(), "acct".into(), None);
    assert_eq!(p.name(), "codex");
}

#[tokio::test]
async fn handler_response_incomplete_emits_error_instead_of_empty_done() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let mut handler = CodexSseHandler::new();
    let out = handler
        .process_line(
            r#"data: {"type":"response.incomplete","response":{"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"}}}"#,
            &tx,
        )
        .await;
    assert!(matches!(out, SseLineOutcome::Done));
    match rx.recv().await.unwrap() {
        StreamEvent::Error(e) => assert!(e.contains("max_output_tokens"), "{e}"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn parse_sse_response_failed_returns_error_instead_of_empty_success() {
    let err = CodexProvider::parse_sse_response(
        r#"data: {"type":"response.failed","response":{"status":"failed","error":{"type":"server_error","message":"boom"}}}"#,
    )
    .unwrap_err();
    assert!(err.to_string().contains("server_error"), "{err}");
    assert!(err.to_string().contains("boom"), "{err}");
}

#[test]
fn build_request_body_includes_max_output_tokens() {
    let messages = vec![Message::system("Sys"), Message::user("U")];
    let body = CodexProvider::build_request_body(&req(&messages, &[], "gpt-5.2", None));
    assert_eq!(body["max_output_tokens"], 1024);
}
