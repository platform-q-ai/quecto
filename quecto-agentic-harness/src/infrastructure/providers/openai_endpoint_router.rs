// #1066: per-request endpoint routing for API-key-authenticated OpenAI
// providers.
//
// OpenAI's documentation routes reasoning models to the Responses API —
// Chat Completions rejects reasoning + function tools with HTTP 400
// ("Function tools with reasoning_effort are not supported ... Please use
// /v1/responses instead"), and our Chat Completions adapter never transmits
// a configured effort. Non-reasoning models stay on Chat Completions
// exactly as today.

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::domain::error::DomainError;
use crate::domain::message::LlmResponse;
use crate::domain::provider::{ChatRequest, LlmProvider, StreamEvent};

/// Routes each request to Chat Completions or the Responses API per
/// OpenAI's documented endpoint rules (#1066).
#[derive(Debug)]
pub struct OpenAiEndpointRouter {
    provider_name: String,
    chat_completions: Arc<dyn LlmProvider>,
    responses: Arc<dyn LlmProvider>,
    /// Bare model ids that are reasoning models per the model registry.
    reasoning_model_ids: HashSet<String>,
}

impl OpenAiEndpointRouter {
    pub fn new(
        provider_name: String,
        chat_completions: Arc<dyn LlmProvider>,
        responses: Arc<dyn LlmProvider>,
        reasoning_model_ids: HashSet<String>,
    ) -> Self {
        Self {
            provider_name,
            chat_completions,
            responses,
            reasoning_model_ids,
        }
    }

    /// A reasoning model always uses the Responses API (with tools it is
    /// mandatory; without tools it is still the only endpoint where a
    /// configured `reasoning.effort` is transmitted); everything else keeps
    /// Chat Completions.
    fn select(&self, request: &ChatRequest<'_>) -> &Arc<dyn LlmProvider> {
        if self.reasoning_model_ids.contains(request.model) {
            &self.responses
        } else {
            &self.chat_completions
        }
    }
}

impl LlmProvider for OpenAiEndpointRouter {
    fn name(&self) -> &str {
        &self.provider_name
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn chat<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        self.select(&request).chat(request)
    }

    fn chat_stream<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        self.select(&request).chat_stream(request)
    }

    fn chat_stream_incremental<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = tokio::sync::mpsc::Receiver<StreamEvent>> + Send + 'a>> {
        self.select(&request).chat_stream_incremental(request)
    }
}
