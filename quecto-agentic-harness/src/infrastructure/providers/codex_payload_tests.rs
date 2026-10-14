//! Payload budget tests through assembled and incremental Responses parsers.
use super::{CodexProvider, CodexSseHandler};
use crate::domain::inference::value_objects::provider::StreamEvent;
use crate::infrastructure::providers::sse_common::{SseHandler, SseLineOutcome};

#[tokio::test]
async fn s1_sse_payload_budget_family() {
    let chunk = format!(
        "{}{}",
        serde_json::json!({"type":"error","error":{"code":"insufficient_quota","message":"secret-canary"}}),
        " ".repeat(4096)
    );
    let err = CodexProvider::parse_sse_response(&format!("data: {chunk}\n")).unwrap_err();
    assert!(
        err.to_string()
            .contains("HTTP 502 from Codex: class=server; refusal=none")
    );
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let mut handler = CodexSseHandler::new();
    assert!(matches!(
        handler.process_line(&format!("data: {chunk}"), &tx).await,
        SseLineOutcome::Done
    ));
    assert!(
        matches!(rx.recv().await, Some(StreamEvent::Error(m)) if m == "HTTP 502 from Codex: class=server; refusal=none")
    );
    let malformed = CodexProvider::parse_sse_response("data: {not json secret-canary\n")
        .unwrap_err()
        .to_string();
    assert!(!malformed.contains("secret-canary"));
    assert!(malformed.contains("ended without completion"));
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
        StreamEvent::Error(e) => assert!(e == "Codex stream incomplete", "{e}"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn parse_sse_response_failed_returns_error_instead_of_empty_success() {
    let err = CodexProvider::parse_sse_response(
        r#"data: {"type":"response.failed","response":{"status":"failed","error":{"type":"server_error","message":"boom"}}}"#,
    )
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("HTTP 500 from Codex: class=server; refusal=none"),
        "{err}"
    );
    assert!(!err.to_string().contains("boom"), "{err}");
}

#[test]
fn parse_sse_response_string_error_event_keeps_its_text() {
    let err =
        CodexProvider::parse_sse_response(r#"data: {"type":"error","error":"model crashed"}"#)
            .unwrap_err();
    assert!(
        err.to_string()
            .contains("HTTP 502 from Codex: class=server; refusal=none"),
        "{err}"
    );
    assert!(!err.to_string().contains("model crashed"), "{err}");
}
