// Region-coverage tests for anthropic_sse.rs.
//
// Focuses on the incremental streaming path (dispatch_sse_event,
// AnthropicSseHandler, emit_tool_call_start/end) and the pure
// SseAccumulator branches that are not exercised elsewhere.
// No sockets / no network: events are constructed in-memory and the
// channel is drained synchronously.

use super::*;
use crate::domain::message::{StopReason, ThinkingBlock};
use crate::domain::provider::StreamEvent;
use crate::infrastructure::providers::sse_common::{SseHandler, SseLineOutcome};

fn channel() -> (
    tokio::sync::mpsc::Sender<StreamEvent>,
    tokio::sync::mpsc::Receiver<StreamEvent>,
) {
    tokio::sync::mpsc::channel(64)
}

fn drain(rx: &mut tokio::sync::mpsc::Receiver<StreamEvent>) -> Vec<StreamEvent> {
    let mut out = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        out.push(ev);
    }
    out
}

// --- stream_event_from_delta -------------------------------------------------

#[test]
fn stream_event_text_delta_non_empty() {
    let delta = serde_json::json!({"type": "text_delta", "text": "hi"});
    match stream_event_from_delta(&delta) {
        Some(StreamEvent::TextDelta(t)) => assert_eq!(t, "hi"),
        other => panic!("expected TextDelta, got {:?}", other),
    }
}

#[test]
fn stream_event_text_delta_empty_is_none() {
    let delta = serde_json::json!({"type": "text_delta", "text": ""});
    assert!(stream_event_from_delta(&delta).is_none());
}

#[test]
fn stream_event_thinking_delta_non_empty() {
    let delta = serde_json::json!({"type": "thinking_delta", "thinking": "reason"});
    match stream_event_from_delta(&delta) {
        Some(StreamEvent::ThinkingDelta(t)) => assert_eq!(t, "reason"),
        other => panic!("expected ThinkingDelta, got {:?}", other),
    }
}

#[test]
fn stream_event_thinking_delta_empty_is_none() {
    let delta = serde_json::json!({"type": "thinking_delta", "thinking": ""});
    assert!(stream_event_from_delta(&delta).is_none());
}

#[test]
fn stream_event_input_json_delta_non_empty() {
    let delta = serde_json::json!({"type": "input_json_delta", "partial_json": "{}"});
    match stream_event_from_delta(&delta) {
        Some(StreamEvent::ToolCallDelta(j)) => assert_eq!(j, "{}"),
        other => panic!("expected ToolCallDelta, got {:?}", other),
    }
}

#[test]
fn stream_event_input_json_delta_empty_is_none() {
    let delta = serde_json::json!({"type": "input_json_delta", "partial_json": ""});
    assert!(stream_event_from_delta(&delta).is_none());
}

#[test]
fn stream_event_signature_delta_is_none() {
    let delta = serde_json::json!({"type": "signature_delta", "signature": "sig"});
    assert!(stream_event_from_delta(&delta).is_none());
}

#[test]
fn stream_event_unknown_type_is_none() {
    let delta = serde_json::json!({"type": "something_else"});
    assert!(stream_event_from_delta(&delta).is_none());
}

#[test]
fn stream_event_missing_type_is_none() {
    let delta = serde_json::json!({"text": "no type field"});
    assert!(stream_event_from_delta(&delta).is_none());
}

// --- SseAccumulator block start ----------------------------------------------

#[test]
fn block_start_tool_use_sets_state() {
    let mut acc = SseAccumulator::default();
    acc.handle_block_start(
        &serde_json::json!({"content_block": {"type": "tool_use", "id": "tu1", "name": "bash"}}),
    );
    assert!(acc.in_tool_input);
    assert_eq!(acc.current_tool_id, "tu1");
    assert_eq!(acc.current_tool_name, "bash");
}

#[test]
fn block_start_redacted_thinking_captures_data() {
    let mut acc = SseAccumulator::default();
    acc.handle_block_start(
        &serde_json::json!({"content_block": {"type": "redacted_thinking", "data": "opaque"}}),
    );
    acc.handle_block_stop();
    let blocks = acc.thinking_blocks();
    assert_eq!(blocks.len(), 1);
    match &blocks[0] {
        ThinkingBlock::Redacted { data } => assert_eq!(data, "opaque"),
        other => panic!("expected Redacted, got {:?}", other),
    }
}

#[test]
fn block_start_redacted_thinking_empty_data_dropped() {
    let mut acc = SseAccumulator::default();
    acc.handle_block_start(&serde_json::json!({"content_block": {"type": "redacted_thinking"}}));
    acc.handle_block_stop();
    assert!(acc.thinking_blocks().is_empty());
}

#[test]
fn block_start_unknown_type_ignored() {
    let mut acc = SseAccumulator::default();
    acc.handle_block_start(&serde_json::json!({"content_block": {"type": "image"}}));
    assert!(!acc.in_tool_input);
}

// --- SseAccumulator block delta / stop ---------------------------------------

#[test]
fn block_delta_unknown_type_ignored() {
    let mut acc = SseAccumulator::default();
    acc.handle_block_delta(&serde_json::json!({"delta": {"type": "mystery"}}));
    // Nothing accumulated; into_response yields no content.
    assert!(acc.into_response().content.is_none());
}

#[test]
fn block_stop_normal_thinking_with_only_signature() {
    let mut acc = SseAccumulator::default();
    acc.handle_block_start(&serde_json::json!({"content_block": {"type": "thinking"}}));
    acc.handle_block_delta(
        &serde_json::json!({"delta": {"type": "signature_delta", "signature": "s"}}),
    );
    acc.handle_block_stop();
    // Whitespace-only thinking but non-empty signature => block kept.
    assert_eq!(acc.thinking_blocks().len(), 1);
}

#[test]
fn block_stop_empty_thinking_dropped() {
    let mut acc = SseAccumulator::default();
    acc.handle_block_start(&serde_json::json!({"content_block": {"type": "thinking"}}));
    acc.handle_block_delta(
        &serde_json::json!({"delta": {"type": "thinking_delta", "thinking": "   "}}),
    );
    acc.handle_block_stop();
    assert!(acc.thinking_blocks().is_empty());
}

#[test]
fn block_stop_without_open_block_is_noop() {
    let mut acc = SseAccumulator::default();
    acc.handle_block_stop();
    assert!(acc.into_response().tool_calls.is_empty());
}

// --- usage / message handling ------------------------------------------------

#[test]
fn message_start_populates_usage_fields() {
    let mut acc = SseAccumulator::default();
    acc.handle_message_start(&serde_json::json!({
        "message": {"usage": {
            "input_tokens": 7, "output_tokens": 3,
            "cache_read_input_tokens": 2, "cache_creation_input_tokens": 1
        }}
    }));
    let resp = acc.into_response();
    let usage = resp.usage.expect("usage");
    assert_eq!(usage.prompt_tokens, 7);
    assert_eq!(usage.completion_tokens, 3);
    assert_eq!(usage.cache_read_tokens, Some(2));
    assert_eq!(usage.cache_write_tokens, Some(1));
}

#[test]
fn message_start_without_usage_is_noop() {
    let mut acc = SseAccumulator::default();
    acc.handle_message_start(&serde_json::json!({"message": {}}));
    assert!(acc.into_response().usage.is_none());
}

#[test]
fn message_delta_sets_stop_reason_and_tokens() {
    let mut acc = SseAccumulator::default();
    acc.handle_message_delta(&serde_json::json!({
        "delta": {"stop_reason": "end_turn"},
        "usage": {"output_tokens": 9, "input_tokens": 4}
    }));
    let resp = acc.into_response();
    assert_eq!(resp.stop_reason, Some(StopReason::EndTurn));
    let usage = resp.usage.expect("usage");
    assert_eq!(usage.completion_tokens, 9);
    assert_eq!(usage.prompt_tokens, 4);
}

#[test]
fn into_response_empty_yields_none_content_and_usage() {
    let resp = SseAccumulator::default().into_response();
    assert!(resp.content.is_none());
    assert!(resp.usage.is_none());
    assert!(resp.tool_calls.is_empty());
}

// --- remap_tool_name ---------------------------------------------------------

#[test]
fn remap_tool_name_no_defs_passthrough() {
    let acc = SseAccumulator::default();
    assert_eq!(acc.remap_tool_name("Read"), "Read");
}

#[test]
fn remap_tool_name_with_defs_maps_and_passes_through() {
    use std::borrow::Cow;
    let defs = vec![crate::domain::tool::ToolDefinition {
        name: Cow::Borrowed("read"),
        description: Cow::Borrowed("read a file"),
        parameters_schema: Cow::Borrowed("{}"),
    }];
    let acc = SseAccumulator::with_tool_defs(defs);
    assert_eq!(acc.remap_tool_name("Read"), "read");
    // Unknown canonical name passes through unchanged (logs debug).
    assert_eq!(acc.remap_tool_name("Unknown"), "Unknown");
}

// --- dispatch_sse_event (async path) -----------------------------------------

#[tokio::test]
async fn dispatch_content_block_start_emits_tool_call_start() {
    let (tx, mut rx) = channel();
    let mut acc = SseAccumulator::default();
    let chunk =
        serde_json::json!({"content_block": {"type": "tool_use", "id": "tu1", "name": "bash"}});
    let done = dispatch_sse_event("content_block_start", &chunk, &mut acc, &tx).await;
    assert!(!done);
    let events = drain(&mut rx);
    assert!(matches!(
        events.first(),
        Some(StreamEvent::ToolCallStart { name, .. }) if name == "bash"
    ));
}

#[tokio::test]
async fn dispatch_redacted_thinking_start_emits_live_placeholder_without_data() {
    let (tx, mut rx) = channel();
    let mut acc = SseAccumulator::default();
    let chunk = serde_json::json!({"content_block": {"type": "redacted_thinking", "data": "opaque-private"}});
    let done = dispatch_sse_event("content_block_start", &chunk, &mut acc, &tx).await;
    assert!(!done);
    let events = drain(&mut rx);
    assert!(
        matches!(events.first(), Some(StreamEvent::ThinkingDelta(t)) if t == "[redacted thinking]")
    );
}

#[tokio::test]
async fn dispatch_content_block_delta_emits_text_and_accumulates() {
    let (tx, mut rx) = channel();
    let mut acc = SseAccumulator::default();
    let chunk = serde_json::json!({"delta": {"type": "text_delta", "text": "hi"}});
    assert!(!dispatch_sse_event("content_block_delta", &chunk, &mut acc, &tx).await);
    let events = drain(&mut rx);
    assert!(matches!(events.first(), Some(StreamEvent::TextDelta(t)) if t == "hi"));
    assert_eq!(acc.into_response().content.as_deref(), Some("hi"));
}

#[tokio::test]
async fn dispatch_content_block_stop_emits_tool_call_end() {
    let (tx, mut rx) = channel();
    let mut acc = SseAccumulator::default();
    acc.handle_block_start(
        &serde_json::json!({"content_block": {"type": "tool_use", "id": "tu1", "name": "bash"}}),
    );
    acc.handle_block_delta(
        &serde_json::json!({"delta": {"type": "input_json_delta", "partial_json": "{}"}}),
    );
    assert!(
        !dispatch_sse_event(
            "content_block_stop",
            &serde_json::Value::Null,
            &mut acc,
            &tx
        )
        .await
    );
    let events = drain(&mut rx);
    assert!(matches!(
        events.first(),
        Some(StreamEvent::ToolCallEnd { name, arguments, .. }) if name == "bash" && arguments == "{}"
    ));
}

#[tokio::test]
async fn dispatch_message_start_and_delta_no_events() {
    let (tx, mut rx) = channel();
    let mut acc = SseAccumulator::default();
    let start = serde_json::json!({"message": {"usage": {"input_tokens": 1}}});
    assert!(!dispatch_sse_event("message_start", &start, &mut acc, &tx).await);
    let delta = serde_json::json!({"delta": {"stop_reason": "end_turn"}});
    assert!(!dispatch_sse_event("message_delta", &delta, &mut acc, &tx).await);
    assert!(drain(&mut rx).is_empty());
}

#[tokio::test]
async fn dispatch_message_stop_emits_done_and_returns_true() {
    let (tx, mut rx) = channel();
    let mut acc = SseAccumulator::default();
    let done = dispatch_sse_event("message_stop", &serde_json::Value::Null, &mut acc, &tx).await;
    assert!(done);
    let events = drain(&mut rx);
    assert!(matches!(events.first(), Some(StreamEvent::Done(_))));
}

#[tokio::test]
async fn dispatch_unknown_event_is_noop() {
    let (tx, mut rx) = channel();
    let mut acc = SseAccumulator::default();
    assert!(!dispatch_sse_event("ping", &serde_json::Value::Null, &mut acc, &tx).await);
    assert!(drain(&mut rx).is_empty());
}

// --- emit_tool_call_start / end edge branches --------------------------------

#[tokio::test]
async fn emit_tool_call_start_skips_when_not_in_tool_input() {
    let (tx, mut rx) = channel();
    let acc = SseAccumulator::default();
    emit_tool_call_start(&acc, &tx).await;
    assert!(drain(&mut rx).is_empty());
}

#[tokio::test]
async fn emit_tool_call_end_skips_when_not_in_tool_input() {
    let (tx, mut rx) = channel();
    let acc = SseAccumulator::default();
    emit_tool_call_end(&acc, &tx).await;
    assert!(drain(&mut rx).is_empty());
}

// --- AnthropicSseHandler -----------------------------------------------------

#[tokio::test]
async fn handler_processes_event_and_data_lines() {
    let (tx, mut rx) = channel();
    let mut handler = AnthropicSseHandler::new(None);
    assert!(matches!(
        handler
            .process_line("event: content_block_delta", &tx)
            .await,
        SseLineOutcome::Continue
    ));
    let outcome = handler
        .process_line(
            "data: {\"delta\":{\"type\":\"text_delta\",\"text\":\"yo\"}}",
            &tx,
        )
        .await;
    assert!(matches!(outcome, SseLineOutcome::Continue));
    let events = drain(&mut rx);
    assert!(matches!(events.first(), Some(StreamEvent::TextDelta(t)) if t == "yo"));
}

#[tokio::test]
async fn handler_invalid_json_data_does_not_panic() {
    let (tx, mut rx) = channel();
    let mut handler = AnthropicSseHandler::new(None);
    let outcome = handler.process_line("data: not-json", &tx).await;
    assert!(matches!(outcome, SseLineOutcome::Continue));
    assert!(drain(&mut rx).is_empty());
}

#[tokio::test]
async fn handler_message_stop_returns_done() {
    let (tx, mut rx) = channel();
    let mut handler = AnthropicSseHandler::new(None);
    let _ = handler.process_line("event: message_stop", &tx).await;
    let outcome = handler.process_line("data: {}", &tx).await;
    assert!(matches!(outcome, SseLineOutcome::Done));
    let events = drain(&mut rx);
    assert!(matches!(events.first(), Some(StreamEvent::Done(_))));
}

#[tokio::test]
async fn handler_on_empty_eof_emits_error() {
    let (tx, mut rx) = channel();
    let mut handler = AnthropicSseHandler::new(None);
    handler.on_eof(&tx).await;
    let events = drain(&mut rx);
    match events.first() {
        Some(StreamEvent::Error(e)) => assert!(e.contains("ended without completion"), "{e}"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn handler_with_tool_defs_remaps_name() {
    use std::borrow::Cow;
    let defs = vec![crate::domain::tool::ToolDefinition {
        name: Cow::Borrowed("read"),
        description: Cow::Borrowed("read a file"),
        parameters_schema: Cow::Borrowed("{}"),
    }];
    let (tx, mut rx) = channel();
    let mut handler = AnthropicSseHandler::new(Some(defs));
    let _ = handler
        .process_line("event: content_block_start", &tx)
        .await;
    let _ = handler
        .process_line(
            "data: {\"content_block\":{\"type\":\"tool_use\",\"id\":\"t\",\"name\":\"Read\"}}",
            &tx,
        )
        .await;
    let events = drain(&mut rx);
    assert!(matches!(
        events.first(),
        Some(StreamEvent::ToolCallStart { name, .. }) if name == "read"
    ));
}

#[tokio::test]
async fn anthropic_sse_handler_test_accessors_cover_new_and_into_response() {
    let mut handler = AnthropicSseHandler::new_for_test(None);
    let (tx, mut rx) = channel();
    assert!(matches!(
        handler
            .process_line(r#"event: content_block_delta"#, &tx)
            .await,
        SseLineOutcome::Continue
    ));
    assert!(matches!(
        handler
            .process_line(r#"data: {"delta":{"type":"text_delta","text":"hi"}}"#, &tx)
            .await,
        SseLineOutcome::Continue
    ));
    assert!(matches!(rx.try_recv(), Ok(StreamEvent::TextDelta(text)) if text == "hi"));
    let resp = handler.into_response();
    assert_eq!(resp.content.as_deref(), Some("hi"));
}

#[tokio::test]
async fn handler_error_event_emits_error_instead_of_empty_done() {
    let (tx, mut rx) = channel();
    let mut handler = AnthropicSseHandler::new(None);
    let _ = handler.process_line("event: error", &tx).await;
    let outcome = handler
        .process_line(
            r#"data: {"error":{"type":"overloaded_error","message":"Overloaded"}}"#,
            &tx,
        )
        .await;
    assert!(matches!(outcome, SseLineOutcome::Done));
    match rx.recv().await.unwrap() {
        StreamEvent::Error(e) => assert!(e.contains("overloaded_error"), "{e}"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn parse_sse_response_error_event_returns_error() {
    let err = AnthropicProvider::parse_sse_response(
        "event: error\ndata: {\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n",
        None,
    )
    .unwrap_err();
    assert!(err.to_string().contains("overloaded_error"), "{err}");
}

#[test]
fn parse_sse_response_empty_or_truncated_stream_returns_error() {
    // No message_stop and no observable output → provider error, not Ok(empty).
    let err =
        AnthropicProvider::parse_sse_response(": keepalive\ndata: {not json\n", None).unwrap_err();
    assert!(
        err.to_string().contains("ended without completion"),
        "{err}"
    );
}

#[test]
fn parse_sse_response_empty_max_tokens_stop_preserves_stop_reason() {
    // Terminal message_stop with no content: parse succeeds (loop guard surfaces
    // the empty turn) and the max_tokens stop reason is preserved.
    let sse = "event: message_delta\n\
               data: {\"delta\":{\"stop_reason\":\"max_tokens\"}}\n\
               event: message_stop\n\
               data: {}\n";
    let resp = AnthropicProvider::parse_sse_response(sse, None).unwrap();
    assert!(resp.content.is_none());
    assert_eq!(
        resp.stop_reason,
        Some(crate::domain::message::StopReason::MaxTokens)
    );
}

#[test]
fn thinking_and_signature_accumulation_are_capped() {
    let oversized = "x".repeat(
        crate::infrastructure::providers::openai::openai_sse_parser::MAX_OPENAI_SSE_REASONING_BYTES
            + 1,
    );
    let mut acc = SseAccumulator::default();
    acc.handle_block_start(&serde_json::json!({"content_block": {"type": "thinking"}}));
    acc.handle_block_delta(
        &serde_json::json!({"delta": {"type": "thinking_delta", "thinking": oversized}}),
    );
    acc.handle_block_delta(
        &serde_json::json!({"delta": {"type": "signature_delta", "signature": oversized}}),
    );
    acc.handle_block_stop();
    assert!(
        acc.thinking_blocks().is_empty(),
        "oversized thinking/signature deltas must not be accumulated or persisted"
    );
}

#[test]
fn oversized_thinking_delta_does_not_emit_live_event() {
    let oversized = "x".repeat(
        crate::infrastructure::providers::openai::openai_sse_parser::MAX_OPENAI_SSE_REASONING_BYTES
            + 1,
    );
    let delta = serde_json::json!({"type": "thinking_delta", "thinking": oversized});
    assert!(
        stream_event_from_delta(&delta).is_none(),
        "oversized live thinking delta must not bypass the accumulator cap"
    );
}
