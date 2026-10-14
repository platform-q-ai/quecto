//! Error-body admission budgets do not restrict ordinary SSE framing.
use super::*;
use crate::domain::inference::value_objects::provider::StreamEvent;
use crate::infrastructure::providers::sse_common::{SseHandler, SseLineOutcome};

fn channel() -> (
    tokio::sync::mpsc::Sender<StreamEvent>,
    tokio::sync::mpsc::Receiver<StreamEvent>,
) {
    tokio::sync::mpsc::channel(64)
}

#[tokio::test]
async fn s1_sse_payload_budget_family() {
    let oversized = format!(
        "{}{}",
        serde_json::json!({"error":{"type":"billing_error","message":"secret-canary"}}),
        " ".repeat(4096)
    );
    for data in [&oversized, "{not json secret-canary"] {
        let err =
            AnthropicProvider::parse_sse_response(&format!("event: error\ndata: {data}\n"), None)
                .unwrap_err()
                .to_string();
        assert!(err.contains("HTTP 502 from Anthropic: class=server; refusal=none"));
        assert!(!err.contains("secret-canary"));
        let (tx, mut rx) = channel();
        let mut handler = AnthropicSseHandler::new(None);
        handler.process_line("event: error", &tx).await;
        assert!(matches!(
            handler.process_line(&format!("data: {data}"), &tx).await,
            SseLineOutcome::Done
        ));
        assert!(
            matches!(rx.recv().await, Some(StreamEvent::Error(m)) if m == "HTTP 502 from Anthropic: class=server; refusal=none")
        );
    }
}

#[tokio::test]
async fn anthropic_error_budget_boundary_preserves_ordinary_framing() {
    let core = r#"{"error":{"type":"billing_error","message":"secret-canary"}}"#;
    for (len, marker) in [
        (4096, "HTTP 402 from Anthropic: class=billing; refusal=none"),
        (4097, "HTTP 502 from Anthropic: class=server; refusal=none"),
    ] {
        let data = format!("{core}{}", " ".repeat(len - core.len()));
        let error =
            AnthropicProvider::parse_sse_response(&format!("event: error\ndata: {data}\n"), None)
                .unwrap_err()
                .to_string();
        assert!(error.ends_with(marker));
        assert!(!error.contains("secret-canary"));
        let (tx, mut rx) = channel();
        let mut handler = AnthropicSseHandler::new(None);
        handler.process_line("event: error", &tx).await;
        assert!(matches!(
            handler.process_line(&format!("data: {data}"), &tx).await,
            SseLineOutcome::Done
        ));
        assert!(matches!(rx.recv().await, Some(StreamEvent::Error(message)) if message == marker));
    }
    let text = "x".repeat(4097);
    let data = serde_json::json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":text}}).to_string();
    let response = AnthropicProvider::parse_sse_response(
        &format!("event: content_block_delta\ndata: {data}\n\nevent: message_stop\ndata: {{}}\n"),
        None,
    )
    .unwrap();
    assert_eq!(response.content.as_deref(), Some(text.as_str()));
    let (tx, mut rx) = channel();
    let mut handler = AnthropicSseHandler::new(None);
    handler
        .process_line("event: content_block_delta", &tx)
        .await;
    assert!(matches!(
        handler.process_line(&format!("data: {data}"), &tx).await,
        SseLineOutcome::Continue
    ));
    assert!(matches!(rx.recv().await, Some(StreamEvent::TextDelta(delta)) if delta == text));
}
