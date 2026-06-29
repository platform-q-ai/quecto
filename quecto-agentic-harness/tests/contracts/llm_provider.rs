//! Contract tests for the `LlmProvider` port.
//!
//! Real providers (AnthropicProvider, OpenAiProvider, CodexProvider) require
//! network access and API keys, so we drive the contract through a minimal
//! inline adapter. This verifies the port's shape: `name()` + `chat()` +
//! default delegation of `chat_stream` to `chat`.
//!
//! Provider-specific behaviour (error classification, SSE parsing, retry
//! policy) lives in each adapter's own unit tests.

use quecto::domain::error::DomainError;
use quecto::domain::message::{LlmResponse, StopReason};
use quecto::domain::provider::{ChatRequest, LlmProvider};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug)]
struct CountingProvider {
    name: String,
    calls: AtomicUsize,
}

impl CountingProvider {
    fn new() -> Self {
        Self {
            name: "counting".into(),
            calls: AtomicUsize::new(0),
        }
    }
    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl LlmProvider for CountingProvider {
    fn name(&self) -> &str {
        &self.name
    }
    fn chat<'a>(
        &'a self,
        _request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            Ok(LlmResponse {
                content: Some("ok".into()),
                tool_calls: vec![],
                usage: None,
                stop_reason: Some(StopReason::EndTurn),
                thinking_blocks: vec![],
            })
        })
    }
}

fn request<'a>() -> ChatRequest<'a> {
    ChatRequest {
        messages: &[],
        tools: &[],
        model: "test-model",
        max_tokens: 100,
        temperature: 0.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    }
}

#[tokio::test]
async fn name_is_exposed() {
    let p: Arc<dyn LlmProvider> = Arc::new(CountingProvider::new());
    assert_eq!(p.name(), "counting");
}

#[tokio::test]
async fn chat_returns_llm_response() {
    let p: Arc<dyn LlmProvider> = Arc::new(CountingProvider::new());
    let r = p.chat(request()).await.unwrap();
    assert_eq!(r.content.as_deref(), Some("ok"));
    assert_eq!(r.stop_reason, Some(StopReason::EndTurn));
}

#[tokio::test]
async fn chat_stream_default_delegates_to_chat() {
    // Contract: the default `chat_stream` implementation delegates to `chat`.
    // A provider that doesn't override must still respond to `chat_stream`.
    let inner = Arc::new(CountingProvider::new());
    let p: Arc<dyn LlmProvider> = inner.clone();

    let r = p.chat_stream(request()).await.unwrap();
    assert_eq!(r.content.as_deref(), Some("ok"));
    assert_eq!(
        inner.call_count(),
        1,
        "chat_stream must delegate to chat when not overridden"
    );
}
