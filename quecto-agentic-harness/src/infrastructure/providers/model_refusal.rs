//! #2435: the decorator that reports a provider's refusal of a model for
//! the account or auth mode in use. It wraps the router: a request whose
//! reply is a definitive refusal (`model_refusal`) is recorded against
//! the provider and model the router sent it to, so the catalogue stops
//! offering that model while the refusal is held; a reply the provider
//! serves for it releases it. Every reply passes through unchanged.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use crate::application::catalogue::ports::ModelRefusalSink;
use crate::application::providers::ports::{ChatRequest, LlmProvider, RouteCheck};
use crate::domain::catalogue::value_objects::catalogue::ModelRef;
use crate::domain::conversation::value_objects::message::LlmResponse;
use crate::domain::error::DomainError;
use crate::domain::inference::services::provider_error::model_refusal;
use crate::domain::inference::value_objects::provider::{ModelRoute, StreamEvent, route_model};

#[derive(Debug)]
pub struct RefusalRecordingProvider {
    inner: Arc<dyn LlmProvider>,
    sink: Arc<dyn ModelRefusalSink>,
    /// How long a refusal is held (`providers.model_refusal_ttl_secs`).
    held_for: Duration,
}

impl RefusalRecordingProvider {
    pub fn new(
        inner: Arc<dyn LlmProvider>,
        sink: Arc<dyn ModelRefusalSink>,
        held_for: Duration,
    ) -> Self {
        Self {
            inner,
            sink,
            held_for,
        }
    }

    /// The provider and model a request for `model` goes to, by the
    /// router's own routing rule over its providers; `None` when it routes
    /// nowhere.
    fn reached(&self, model: &str) -> Option<ModelRef> {
        let order = self.inner.route_order();
        let names: Vec<&str> = order.iter().map(String::as_str).collect();
        match route_model(model, &names) {
            ModelRoute::To { provider, model } => ModelRef::parse(provider, model).ok(),
            ModelRoute::UnknownProvider { .. } | ModelRoute::NoProviders => None,
        }
    }

    /// Record `error` against the model a request for `model` reached when
    /// it is a definitive refusal; any other error records nothing.
    fn observe(&self, model: &str, error: &DomainError) {
        if let Some(reference) = self.reached(model) {
            record(self.sink.as_ref(), &reference, error, self.held_for);
        }
    }

    /// Release the refusal of the model a request for `model` reached: the
    /// provider served it.
    fn release(&self, model: &str) {
        if let Some(reference) = self.reached(model) {
            release(self.sink.as_ref(), &reference);
        }
    }

    /// Wrap a reply future so a refusal it ends with is observed, and a
    /// reply it serves releases one.
    fn observed<'a>(
        &'a self,
        model: &'a str,
        reply: Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        Box::pin(async move {
            let result = reply.await;
            match &result {
                Ok(_) => self.release(model),
                Err(error) => self.observe(model, error),
            }
            result
        })
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
        let model = request.model;
        self.observed(model, self.inner.chat(request))
    }

    fn chat_stream<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        let model = request.model;
        self.observed(model, self.inner.chat_stream(request))
    }

    fn chat_stream_incremental<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = tokio::sync::mpsc::Receiver<StreamEvent>> + Send + 'a>> {
        // The events are relayed as they come; only the terminal ones are
        // looked at. The relay ends as soon as either side does: a caller
        // that drops its receiver drops the provider's stream with it, so
        // the transport sees it gone and stops (review round 1 M1).
        let reached = self.reached(request.model);
        let sink = self.sink.clone();
        let held_for = self.held_for;
        let stream = self.inner.chat_stream_incremental(request);
        Box::pin(async move {
            let mut events = stream.await;
            let Some(reference) = reached else {
                return events;
            };
            let (tx, rx) = tokio::sync::mpsc::channel(RELAY_CAPACITY);
            tokio::spawn(async move {
                while let Some(event) = next_event(&mut events, &tx).await {
                    match &event {
                        StreamEvent::Error(message) => {
                            let error = DomainError::Provider(message.clone());
                            record(sink.as_ref(), &reference, &error, held_for);
                        }
                        StreamEvent::Done(_) => release(sink.as_ref(), &reference),
                        _ => {}
                    }
                    if tx.send(event).await.is_err() {
                        return;
                    }
                }
            });
            rx
        })
    }
}

/// The relay's buffer: the providers' own stream channels hold 32 events.
const RELAY_CAPACITY: usize = 32;

/// The provider's next event, or `None` once either the provider or the
/// caller has gone (as `codex_replay::next_event` relays).
async fn next_event(
    events: &mut tokio::sync::mpsc::Receiver<StreamEvent>,
    tx: &tokio::sync::mpsc::Sender<StreamEvent>,
) -> Option<StreamEvent> {
    tokio::select! {
        event = events.recv() => event,
        () = tx.closed() => None,
    }
}

/// Release the refusal of `reference`, if one is held: it was served.
fn release(sink: &dyn ModelRefusalSink, reference: &ModelRef) {
    if sink.clear_refusal(reference) {
        tracing::info!(
            model = %reference.qualified_id(),
            "provider served a model it had refused; it is offered again"
        );
    }
}

/// Record `error` against `reference` when it is a definitive refusal and
/// refusals are held at all: a zero hold (`model_refusal_ttl_secs = 0`)
/// records nothing, and the log says the model is still offered.
fn record(
    sink: &dyn ModelRefusalSink,
    reference: &ModelRef,
    error: &DomainError,
    held_for: Duration,
) {
    let Some(reason) = model_refusal(error) else {
        return;
    };
    match held_for.is_zero() {
        true => tracing::warn!(
            model = %reference.qualified_id(),
            reason = %reason,
            "provider refused the model for this account; refusals are not held \
             (providers.model_refusal_ttl_secs = 0), so it is still offered"
        ),
        false => {
            if sink.record_refusal(reference, &reason, held_for) {
                tracing::warn!(
                    model = %reference.qualified_id(),
                    reason = %reason,
                    held_secs = held_for.as_secs(),
                    "provider refused the model for this account; it is not offered while the \
                     refusal is held"
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "model_refusal_tests.rs"]
mod tests;
