//! Capability-local ports of the providers capability (#1960).
//!
//! The agent turn talks to a model through [`LlmProvider`] and re-checks
//! local execution policy through [`RequestAdmission`] before every request
//! (and retry). Infrastructure implements both: the vendor clients and the
//! router, and the admission broker. [`ChatRequest`] is the request they
//! exchange; the pure vocabulary it carries stays in `domain::provider`.
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::domain::error::DomainError;
use crate::domain::message::{LlmResponse, Message};
use crate::domain::provider::{
    CancelFlag, EffortLevel, RequestMetadata, StreamEvent, ThinkingLevel, ToolChoice,
};
use crate::domain::request_observation::{RequestObservation, RequestTrace};
use crate::domain::tool::ToolDefinition;

/// Parameters for a chat request to an LLM provider.
#[derive(Debug, Clone)]
pub struct ChatRequest<'a> {
    /// Local retry diagnostics shared across request wrappers.
    pub trace: Option<Arc<RequestTrace>>,
    /// Local execution policy, rechecked before retries; never sent to providers.
    pub admission: Option<Arc<dyn RequestAdmission>>,
    pub messages: &'a [Message],
    pub tools: &'a [ToolDefinition],
    pub model: &'a str,
    pub max_tokens: u32,
    pub temperature: f32,
    /// Optional session identifier for providers that support prompt caching
    /// keyed by session (e.g. Codex `prompt_cache_key`).
    pub session_id: Option<&'a str>,
    /// Optional tool_choice parameter to control how the model selects tools.
    pub tool_choice: Option<ToolChoice>,
    /// Optional metadata (e.g. user_id for multi-tenant rate limiting).
    pub metadata: Option<RequestMetadata>,
    /// Optional thinking level for extended thinking support.
    /// When set, the Anthropic provider adds a `thinking` parameter to the request.
    /// Use `ThinkingLevel::Adaptive` for Opus 4.6 / Sonnet 4.6 (recommended).
    /// Use `ThinkingLevel::Low/Medium/High/Max` for older models (manual budget).
    pub thinking_level: Option<ThinkingLevel>,
    /// Optional cancellation flag. When `Some`, the provider checks this flag
    /// before processing and returns `DomainError::Provider("request cancelled")`
    /// immediately if it is set. Set `cancel_flag.cancel()` from any thread to
    /// cancel an in-flight request.
    pub cancel_flag: Option<CancelFlag>,
    /// Optional effort level for the `output_config.effort` API parameter.
    /// Controls thinking depth and token spend on Opus 4.6 / Sonnet 4.6.
    /// When `None`, the Anthropic provider defaults to `low` for 4.6 models
    /// (to avoid the API's implicit `high` default); omitted for other models.
    pub effort: Option<EffortLevel>,
}

/// Whether a model id would reach a provider (#2126).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteCheck {
    /// A request for this model has a provider to go to.
    Routable,
    /// A `provider/model` id names a provider this harness is not configured
    /// with; `configured` lists the ones it is.
    UnknownProvider {
        provider: String,
        configured: Vec<String>,
    },
}

/// Port: an LLM provider that can process chat requests.
pub trait LlmProvider: Send + Sync + std::fmt::Debug {
    /// Human-readable provider name (e.g. "openai", "anthropic").
    fn name(&self) -> &str;

    /// Whether a request for `model` would reach a provider. A single
    /// provider takes whatever it is given; a router answers from the
    /// providers it holds, and every decorator around one must forward.
    fn route_check(&self, _model: &str) -> RouteCheck {
        RouteCheck::Routable
    }

    /// Downcast support for introspection (e.g. recovering a concrete
    /// `ProviderRouter` for diagnostics and tests). Implementors that need to be
    /// downcast override this to return `self`; the default returns a reference
    /// that downcasts to nothing useful.
    fn as_any(&self) -> &dyn std::any::Any {
        &()
    }

    /// Send a chat request and return the LLM response.
    ///
    /// The lifetime `'a` ties `&self`, the request's borrowed data, and
    /// the returned future together. This allows wrappers (e.g. routers)
    /// to forward borrowed slices without cloning.
    fn chat<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>>;

    /// Send a streaming chat request and return the assembled response.
    ///
    /// Default implementation delegates to `chat()` (non-streaming).
    /// Providers that support SSE streaming override this method.
    fn chat_stream<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        self.chat(request)
    }

    /// Send a streaming chat request and return a channel that emits
    /// incremental [`StreamEvent`]s as each SSE packet arrives.
    ///
    /// The channel is closed after either a [`StreamEvent::Done`] or
    /// [`StreamEvent::Error`] is sent. Callers should read until the channel
    /// closes or until they receive one of those terminal events.
    ///
    /// Default implementation wraps `chat_stream()` and emits a single
    /// `Done` or `Error` event — no true incremental delivery.
    /// Providers that support byte-stream SSE should override this method.
    fn chat_stream_incremental<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = tokio::sync::mpsc::Receiver<StreamEvent>> + Send + 'a>> {
        let fut = self.chat_stream(request);
        Box::pin(async move {
            let (tx, rx) = tokio::sync::mpsc::channel(32);
            match fut.await {
                Ok(resp) => {
                    let _ = tx.send(StreamEvent::Done(resp)).await;
                }
                Err(e) => {
                    let _ = tx.send(StreamEvent::Error(e.to_string())).await;
                }
            }
            rx
        })
    }
}

/// Admission policy evaluated before each model request. Implementations may
/// consult durable execution state without exposing its storage to the agent.
pub trait RequestAdmission: std::fmt::Debug + Send + Sync {
    fn check(&self) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>>;
}

/// Port: durable accounting of one completed request observation, recorded
/// by the agent turn after each logical request (the swarm's usage ledger
/// implements it).
pub trait RequestAccounting: Send + Sync {
    fn record<'a>(
        &'a self,
        observation: &'a RequestObservation,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + 'a>>;
}

#[cfg(test)]
#[path = "ports_cov_tests.rs"]
mod cov_tests;
