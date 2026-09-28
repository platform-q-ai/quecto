//! #2249 review: a Responses reply ends only at a terminal event. A body
//! that ends without one is a reply cut short — an error classified as a
//! transport cut (retryable `Network`), never a whole reply — and an error
//! chunk in any shape the providers send ends the stream as an error.
use super::*;
use crate::domain::provider_error::{ProviderErrorClass, classify_provider_error};

const TEXT: &str = r#"data: {"type":"response.output_text.delta","delta":"Hi"}"#;
const COMPLETED: &str = r#"data: {"type":"response.completed","response":{}}"#;

/// Feed `lines` to a fresh handler, then end the body; every event sent.
async fn handled(lines: &[&str]) -> Vec<StreamEvent> {
    let (tx, mut rx) = tokio::sync::mpsc::channel(16);
    let mut handler = CodexSseHandler::new();
    let mut ended = false;
    for line in lines {
        if matches!(handler.process_line(line, &tx).await, SseLineOutcome::Done) {
            ended = true;
            break;
        }
    }
    if !ended {
        handler.on_eof(&tx).await;
    }
    drop(tx);
    let mut events = Vec::new();
    while let Some(event) = rx.recv().await {
        events.push(event);
    }
    events
}

fn last_error(events: &[StreamEvent]) -> &str {
    match events.last() {
        Some(StreamEvent::Error(message)) => message,
        other => panic!("expected an error last: {other:?}"),
    }
}

fn is_cut_short(message: &str) -> bool {
    message.contains("ended without completion")
        && classify_provider_error(&DomainError::Provider(message.to_owned()))
            == ProviderErrorClass::Network
}

#[tokio::test]
async fn a_streamed_reply_whose_body_ends_before_its_terminal_event_is_cut_short() {
    for lines in [
        &[TEXT][..],
        &[
            r#"data: {"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","call_id":"c1","name":"bash","arguments":""}}"#,
            r#"data: {"type":"response.function_call_arguments.delta","output_index":0,"delta":"{\"x\":1}"}"#,
        ][..],
    ] {
        let events = handled(lines).await;
        assert!(is_cut_short(last_error(&events)), "{lines:?}: {events:?}");
        assert!(
            !events.iter().any(|e| matches!(e, StreamEvent::Done(_))),
            "{events:?}"
        );
    }
}

#[tokio::test]
async fn a_streamed_reply_ends_whole_only_at_a_terminal_event() {
    for terminal in [COMPLETED, "data: [DONE]"] {
        let events = handled(&[TEXT, terminal]).await;
        assert!(
            matches!(events.last(), Some(StreamEvent::Done(r)) if r.content.as_deref() == Some("Hi")),
            "{terminal}: {events:?}"
        );
    }
}

#[test]
fn a_whole_body_that_ends_before_its_terminal_event_is_cut_short() {
    let Err(DomainError::Provider(message)) = CodexProvider::parse_sse_response(TEXT) else {
        panic!("a body without its terminal event is a provider error");
    };
    assert!(is_cut_short(&message), "{message}");
    let whole = CodexProvider::parse_sse_response(&format!("{TEXT}\n{COMPLETED}\n")).unwrap();
    assert_eq!(whole.content.as_deref(), Some("Hi"));
}

/// An untyped `{"error":…}` chunk (a gateway or compatible server's
/// shape) ends the stream as an error on both reads, its text kept.
#[tokio::test]
async fn an_untyped_error_chunk_ends_the_stream_as_an_error() {
    for chunk in [
        r#"data: {"error":"model crashed"}"#,
        // A `null` type is no type (#2249 review round 2).
        r#"data: {"type":null,"error":{"message":"model crashed"}}"#,
        r#"data: {"error":{"type":"server_error","message":"model crashed"}}"#,
    ] {
        let events = handled(&[TEXT, chunk, COMPLETED]).await;
        let message = last_error(&events);
        assert!(message.contains("Responses stream error"), "{message}");
        assert!(message.contains("model crashed"), "{message}");
        let err = CodexProvider::parse_sse_response(&format!("{TEXT}\n{chunk}\n{COMPLETED}\n"))
            .unwrap_err();
        assert!(err.to_string().contains("model crashed"), "{err}");
    }
}

/// Only an untyped chunk whose `error` is an object or a non-empty string
/// is an error chunk; any other `error` is no error, and a typed event
/// keeps its own meaning.
#[tokio::test]
async fn a_chunk_is_an_error_chunk_only_by_its_allowed_shapes() {
    for chunk in [
        r#"data: {"error":null}"#,
        r#"data: {"error":""}"#,
        r#"data: {"error":42}"#,
        r#"data: {"type":"response.created","error":"not a failure"}"#,
    ] {
        let events = handled(&[TEXT, chunk, COMPLETED]).await;
        assert!(
            matches!(events.last(), Some(StreamEvent::Done(_))),
            "{chunk}: {events:?}"
        );
    }
}

/// OpenAI documents the Responses `error` event with top-level `code` and
/// `message`; both are read, as is the nested `error` form.
#[tokio::test]
async fn an_error_event_reads_its_top_level_code_and_message() {
    let events = handled(&[
        TEXT,
        r#"data: {"type":"error","code":"server_error","message":"The server had an error","param":null,"sequence_number":3}"#,
    ])
    .await;
    let message = last_error(&events);
    assert!(message.contains("code=server_error"), "{message}");
    assert!(message.contains("The server had an error"), "{message}");

    let nested = CodexProvider::format_stream_failure(&serde_json::json!({
        "type": "error",
        "error": {"type": "invalid_request_error", "code": "bad_input", "message": "nested"}
    }))
    .unwrap();
    assert!(nested.contains("type=invalid_request_error"), "{nested}");
    assert!(nested.contains("code=bad_input"), "{nested}");
    assert!(nested.contains("nested"), "{nested}");
}

/// A `response.failed` keeps its established rendering: its nested
/// `code` is not added, so its classification and wording are unchanged.
#[test]
fn a_failed_response_keeps_its_rendering() {
    let failed = CodexProvider::format_stream_failure(&serde_json::json!({
        "type": "response.failed",
        "code": "top_level_ignored",
        "message": "top level ignored",
        "response": {"status": "failed", "error": {"type": "rate_limit_error", "code": "insufficient_quota", "message": "m"}}
    }))
    .unwrap();
    assert_eq!(
        failed,
        "Responses stream response.failed: status=failed: type=rate_limit_error: m"
    );
}

/// A failure's top-level `message` or `code` is never read: only an `error`
/// event carries them there, so a `response.failed` without a nested
/// message shows none.
#[test]
fn a_failed_response_never_reads_top_level_fields() {
    let failed = CodexProvider::format_stream_failure(&serde_json::json!({
        "type": "response.failed",
        "code": "top_level_ignored",
        "message": "top level ignored",
        "response": {"status": "failed", "error": {"type": "server_error"}}
    }))
    .unwrap();
    assert_eq!(
        failed,
        "Responses stream response.failed: status=failed: type=server_error"
    );
}

/// A body with no event at all is an empty stream (#2249 review round 2):
/// retried as one, as before.
#[tokio::test]
async fn a_body_with_no_event_is_an_empty_stream() {
    for lines in [&[][..], &[": keepalive"][..]] {
        let events = handled(lines).await;
        assert_eq!(
            last_error(&events),
            crate::domain::provider_error::EMPTY_STREAM,
            "{lines:?}"
        );
    }
}

/// An untyped `{"error":{}}` names no cause but still ends the stream as
/// an error, before a terminal event would complete it (#2249 review
/// round 3).
#[tokio::test]
async fn an_untyped_empty_error_object_ends_the_stream_as_an_error() {
    let chunk = r#"data: {"error":{}}"#;
    for terminal in [COMPLETED, "data: [DONE]"] {
        let events = handled(&[TEXT, chunk, terminal]).await;
        assert!(
            last_error(&events).starts_with("Responses stream error"),
            "{events:?}"
        );
        assert!(!events.iter().any(|e| matches!(e, StreamEvent::Done(_))));
    }
    let err =
        CodexProvider::parse_sse_response(&format!("{TEXT}\n{chunk}\n{COMPLETED}\n")).unwrap_err();
    assert!(err.to_string().contains("Responses stream error"), "{err}");
}
