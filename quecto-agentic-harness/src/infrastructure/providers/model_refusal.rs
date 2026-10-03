//! #2435: the decorator that reports a provider's refusal of a model for
//! the account or auth mode in use. It wraps the router: a request whose
//! reply is a definitive refusal (`model_refusal`) is recorded against
//! the provider and model the router sent it to, so the catalogue stops
//! offering that model for the rest of the process. Every reply passes
//! through unchanged.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::application::catalogue::ports::ModelRefusalSink;
use crate::application::providers::ports::{ChatRequest, LlmProvider, RouteCheck};
use crate::domain::error::DomainError;
use crate::domain::message::LlmResponse;
use crate::domain::provider::StreamEvent;

#[derive(Debug)]
pub struct RefusalRecordingProvider {
    inner: Arc<dyn LlmProvider>,
    sink: Arc<dyn ModelRefusalSink>,
}

impl RefusalRecordingProvider {
    pub fn new(inner: Arc<dyn LlmProvider>, sink: Arc<dyn ModelRefusalSink>) -> Self {
        Self { inner, sink }
    }
}

impl LlmProvider for RefusalRecordingProvider {
    fn route_check(&self, model: &str) -> RouteCheck {
        self.inner.route_check(model)
    }

    fn route_order(&self) -> Vec<String> {
        self.inner.route_order()
    }

    fn name(&self) -> &str {
        self.inner.name()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn chat<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        let _ = &self.sink;
        self.inner.chat(request)
    }

    fn chat_stream<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        self.inner.chat_stream(request)
    }

    fn chat_stream_incremental<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = tokio::sync::mpsc::Receiver<StreamEvent>> + Send + 'a>> {
        self.inner.chat_stream_incremental(request)
    }
}

#[cfg(test)]
#[path = "model_refusal_tests.rs"]
mod tests;
