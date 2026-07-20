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
#[path = "test_support_tests.rs"]
mod tests;
