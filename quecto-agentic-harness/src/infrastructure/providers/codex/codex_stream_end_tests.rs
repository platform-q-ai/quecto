//! #2249 review: a Responses reply ends only at a terminal event. A body
//! that ends without one is a reply cut short — an error classified as a
//! transport cut (retryable `Network`), never a whole reply — and an error
//! chunk in any shape the providers send ends the stream as an error.
use super::*;
use crate::domain::inference::services::provider_error::{
    ProviderErrorClass, classify_provider_error,
};

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
        // A billing code names the billing class (Refs #2531); the rest is
        // as it was.
        "provider stream failure (billing): Responses stream response.failed: status=failed: type=rate_limit_error: m"
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
        "provider stream failure (server error): Responses stream response.failed: status=failed: type=server_error"
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
            crate::domain::inference::services::provider_error::EMPTY_STREAM,
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

/// #2434: a streamed reply that completed with nothing in it is whole,
/// stopped at `end_turn`; the agent loop, not the stream, decides what it
/// means.
#[tokio::test]
async fn a_streamed_reply_with_nothing_in_it_ends_whole_at_end_turn() {
    let completed =
        r#"data: {"type":"response.completed","response":{"status":"completed","output":[]}}"#;
    let events = handled(&[completed]).await;
    match events.last() {
        Some(StreamEvent::Done(response)) => {
            assert!(response.content.is_none(), "{response:?}");
            assert!(response.tool_calls.is_empty());
            assert_eq!(
                response.stop_reason,
                Some(crate::domain::conversation::value_objects::message::StopReason::EndTurn)
            );
        }
        other => panic!("expected a whole reply: {other:?}"),
    }
}

/// Refs #2531: a typed stream failure renders with the class its code (or,
/// with no code, its type) names on this wire, a billing name first, so the
/// domain classifies it — an overloaded or failing server is retried, never
/// terminal as an unknown failure. A code the wire does not know names
/// nothing, whatever the type says (#2620 review), and no HTTP status is
/// claimed for a stream that arrived as HTTP 200.
#[test]
fn a_typed_stream_failure_renders_the_status_its_name_stands_for() {
    use crate::domain::inference::services::provider_error::{
        ProviderErrorClass, classify_provider_error,
    };
    use crate::domain::shared::error::DomainError;
    use serde_json::json;
    let class = |event: serde_json::Value| {
        let rendered = CodexProvider::format_stream_failure(&event).expect("a failure");
        (
            classify_provider_error(&DomainError::Provider(rendered.clone())),
            rendered,
        )
    };
    for (event, expected) in [
        (
            json!({"type": "error", "error": {"type": "service_unavailable_error", "code": "server_is_overloaded", "message": "m"}}),
            ProviderErrorClass::Server,
        ),
        (
            json!({"type": "response.failed", "response": {"status": "failed", "error": {"type": "server_error", "message": "m"}}}),
            ProviderErrorClass::Server,
        ),
        (
            json!({"type": "error", "code": "server_error", "message": "m"}),
            ProviderErrorClass::Server,
        ),
        (
            json!({"type": "error", "error": {"type": "service_unavailable_error", "message": "m"}}),
            ProviderErrorClass::Server,
        ),
        (
            json!({"type": "error", "code": "rate_limit_exceeded", "message": "m"}),
            ProviderErrorClass::RateLimit,
        ),
        (
            json!({"type": "error", "code": "slow_down"}),
            ProviderErrorClass::RateLimit,
        ),
        (
            json!({"type": "response.failed", "response": {"error": {"type": "requests", "code": "rate_limit_exceeded"}}}),
            ProviderErrorClass::RateLimit,
        ),
        (
            json!({"type": "error", "code": "invalid_api_key", "message": "m"}),
            ProviderErrorClass::Auth,
        ),
        (
            json!({"type": "error", "error": {"type": "invalid_request_error", "message": "m"}}),
            ProviderErrorClass::Client,
        ),
        (
            json!({"type": "error", "code": "insufficient_quota", "message": "m"}),
            ProviderErrorClass::Billing,
        ),
        (
            json!({"type": "error", "code": "usage_limit_reached"}),
            ProviderErrorClass::Billing,
        ),
        (
            json!({"type": "response.failed", "response": {"error": {"type": "server_error", "code": "credit_balance_exhausted"}}}),
            ProviderErrorClass::Billing,
        ),
        (
            json!({"type": "error", "code": "made_up", "message": "m"}),
            ProviderErrorClass::Unknown,
        ),
        // An overloaded backend by its code alone (#2620 review B1).
        (
            json!({"type": "error", "code": "server_is_overloaded", "message": "m"}),
            ProviderErrorClass::Server,
        ),
        (
            json!({"type": "response.failed", "response": {"error": {"code": "server_is_overloaded"}}}),
            ProviderErrorClass::Server,
        ),
        // A billing name in any field is billing, whatever the other says
        // (#2620 review round 4: quecto's conservative choice).
        (
            json!({"type": "error", "error": {"type": "insufficient_quota", "code": "rate_limit_exceeded"}}),
            ProviderErrorClass::Billing,
        ),
        (
            json!({"type": "response.failed", "response": {"error": {"type": "billing_error", "code": "server_error"}}}),
            ProviderErrorClass::Billing,
        ),
        // The code wins; an unknown code names nothing (#2620 review B2).
        (
            json!({"type": "error", "error": {"type": "server_error", "code": "context_length_exceeded"}}),
            ProviderErrorClass::Unknown,
        ),
        (
            json!({"type": "error", "error": {"type": "server_error", "code": "invalid_api_key"}}),
            ProviderErrorClass::Auth,
        ),
    ] {
        let (class, rendered) = class(event.clone());
        assert_eq!(class, expected, "{event} rendered {rendered}");
    }
    let (_, unnamed) = class(json!({"type": "error", "code": "made_up", "message": "m"}));
    assert!(unnamed.starts_with("Responses stream error"), "{unnamed}");
    // No HTTP status is claimed for a classed stream failure (#2620 review B4).
    use crate::domain::inference::services::provider_error::provider_http_status;
    let (_, classed) = class(json!({"type": "error", "code": "server_is_overloaded"}));
    assert_eq!(
        provider_http_status(&DomainError::Provider(classed.clone())),
        None,
        "{classed}"
    );
}

/// #2531 (#2626 review MED3): a Codex overload that arrives mid-stream
/// (`service_unavailable_error` / `server_is_overloaded`), rendered as the
/// wire classes it (`classed_stream_failure`), suspends a member as a
/// recoverable failure, and the next swarm wake re-arms it and owes the run
/// a turn to continue the interrupted work.
#[test]
fn a_codex_overload_mid_stream_suspends_as_recoverable_and_the_next_wake_re_arms() {
    use crate::domain::inference::services::turn_suspension::{
        AutomaticTurns, OwedTurn, SuspensionCause, WakeOutcome,
    };
    use crate::domain::shared::error::DomainError;
    let rendered = CodexProvider::format_stream_failure(&serde_json::json!({
        "type": "error",
        "error": {
            "type": "service_unavailable_error",
            "code": "server_is_overloaded",
            "message": "The server is overloaded"
        }
    }))
    .expect("a failure");
    assert!(
        rendered.starts_with("provider stream failure ("),
        "the class is rendered: {rendered}"
    );
    let mut turns = AutomaticTurns::default();
    turns.suspend_after_failure(&DomainError::Provider(rendered), Some(3));
    assert_eq!(
        turns.suspension().map(|suspension| suspension.cause),
        Some(SuspensionCause::RecoverableProviderFailure)
    );
    assert_eq!(turns.wake(3), WakeOutcome::RearmedAfterRecoverableFailure);
    assert!(turns.allowed());
    assert_eq!(
        turns.take_owed_turn(),
        Some(OwedTurn::AfterRecoverableFailure)
    );
}
