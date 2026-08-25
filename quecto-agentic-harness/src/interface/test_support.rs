//! Shared test utilities for the interface layer.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::domain::catalogue::{
    AuthIdentity, Availability, ModelCapabilities, ModelDescriptor, ModelId, ModelRef, ProviderId,
    TransportKind,
};
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

    fn model_descriptors(&self) -> Option<&[ModelDescriptor]> {
        static MODELS: std::sync::OnceLock<Vec<ModelDescriptor>> = std::sync::OnceLock::new();
        Some(MODELS.get_or_init(|| {
            [
                ("stub", "stub"),
                ("openai", "gpt-5.2"),
                ("anthropic-api", "claude-sonnet-4-6"),
            ]
            .into_iter()
            .map(|(provider, model)| ModelDescriptor {
                reference: ModelRef::new(
                    ProviderId::new(provider).unwrap(),
                    ModelId::new(model).unwrap(),
                ),
                display_name: Some(model.into()),
                transport: TransportKind::OpenAiCompletions,
                auth: AuthIdentity::ApiKey,
                capabilities: ModelCapabilities {
                    input: vec!["text".into()],
                    context_window: 0,
                    max_tokens: 0,
                    context_window_explicit: false,
                    max_tokens_explicit: false,
                    reasoning: false,
                    cost: Default::default(),
                },
                availability: Availability::Runnable,
                base_url: None,
                auth_header: false,
                allow_remote_http: false,
                configured: true,
            })
            .collect()
        }))
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
