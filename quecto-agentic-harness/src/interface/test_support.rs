//! Shared test utilities for the interface layer.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::domain::error::DomainError;
use crate::domain::message::LlmResponse;
use crate::domain::provider::{ChatRequest, LlmProvider};

/// A minimal LLM provider stub that returns a fixed response without
/// making any HTTP calls. Used by REPL and CLI unit tests.
#[derive(Debug)]
pub(crate) struct StubProvider;

impl LlmProvider for StubProvider {
    fn name(&self) -> &str {
        "stub"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn chat(
        &self,
        _request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        Box::pin(async {
            Ok(LlmResponse {
                content: Some("stub response".to_string()),
                tool_calls: vec![],
                usage: None,
                stop_reason: None,
                thinking_blocks: vec![],
            })
        })
    }
}

/// Create an `Arc<dyn LlmProvider>` backed by [`StubProvider`].
pub(crate) fn make_stub_provider() -> Arc<dyn LlmProvider> {
    Arc::new(StubProvider)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_provider_reports_name() {
        let provider = StubProvider;
        assert_eq!(provider.name(), "stub");
    }

    #[tokio::test]
    async fn stub_provider_trait_methods_are_scripted() {
        let provider = make_stub_provider();
        let messages = [];
        let request = ChatRequest {
            messages: &messages,
            tools: &[],
            model: "stub-model",
            max_tokens: 1024,
            temperature: 0.0,
            session_id: None,
            tool_choice: None,
            metadata: None,
            thinking_level: None,
            cancel_flag: None,
            effort: None,
        };

        assert!(provider.as_any().downcast_ref::<StubProvider>().is_some());
        let response = provider.chat(request).await.unwrap();
        assert_eq!(response.content.as_deref(), Some("stub response"));
        assert!(response.tool_calls.is_empty());
        assert!(response.usage.is_none());
        assert!(response.stop_reason.is_none());
        assert!(response.thinking_blocks.is_empty());

        let messages = [];
        let request = ChatRequest {
            messages: &messages,
            tools: &[],
            model: "stub-model",
            max_tokens: 1024,
            temperature: 0.0,
            session_id: None,
            tool_choice: None,
            metadata: None,
            thinking_level: None,
            cancel_flag: None,
            effort: None,
        };
        let streamed = provider.chat_stream(request).await.unwrap();
        assert_eq!(streamed.content.as_deref(), Some("stub response"));

        let messages = [];
        let request = ChatRequest {
            messages: &messages,
            tools: &[],
            model: "stub-model",
            max_tokens: 1024,
            temperature: 0.0,
            session_id: None,
            tool_choice: None,
            metadata: None,
            thinking_level: None,
            cancel_flag: None,
            effort: None,
        };
        let mut rx = provider.chat_stream_incremental(request).await;
        assert!(
            matches!(rx.recv().await, Some(crate::domain::provider::StreamEvent::Done(done)) if done.content.as_deref() == Some("stub response"))
        );
        assert!(rx.recv().await.is_none());
    }
}
