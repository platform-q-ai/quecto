// Provider router: routes ChatRequests to the correct provider based on
// `provider/model-id` syntax, where model-id is opaque. No fallback, no
// cloning, no cooldown.
//
// Replaces FallbackProvider (#370).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::application::providers::ports::{ChatRequest, LlmProvider};
use crate::domain::conversation::value_objects::message::LlmResponse;
use crate::domain::error::DomainError;
use crate::domain::inference::value_objects::provider::StreamEvent;
use crate::domain::inference::value_objects::provider::{ModelRoute, route_model};

/// A provider that routes requests to the correct underlying provider
/// based on `provider/model` syntax. Bare model names (no `/`) are
/// sent to the first provider in the list.
///
/// Unlike the old `FallbackProvider`, this does **not** retry on failure,
/// does **not** clone the conversation, and does **not** track cooldowns.
#[derive(Debug)]
pub struct ProviderRouter {
    providers: Vec<Arc<dyn LlmProvider>>,
}

impl ProviderRouter {
    /// Create a new router from an ordered list of providers.
    /// The first provider is the default for bare model names.
    pub fn new(providers: Vec<Arc<dyn LlmProvider>>) -> Self {
        Self { providers }
    }

    /// Names of the configured providers, in routing order.
    ///
    /// Exposed for introspection (provider construction tests, diagnostics);
    /// the router itself routes by prefix match against these names.
    pub fn provider_names(&self) -> Vec<&str> {
        self.providers.iter().map(|p| p.name()).collect()
    }

    /// Resolve which provider and effective model to use for a request.
    ///
    /// - `provider/model` syntax → match by provider name, strip prefix
    /// - Bare model → first provider in the list
    ///
    /// Lifetimes are decoupled: the provider reference borrows from `self`,
    /// while the bare model borrows from the input `model` string.
    fn resolve<'a, 'b>(
        &'a self,
        model: &'b str,
    ) -> Result<(&'a Arc<dyn LlmProvider>, &'b str), DomainError> {
        let names = self.provider_names();
        match route_model(model, &names) {
            ModelRoute::To { provider, model } => {
                let routed = self
                    .providers
                    .iter()
                    .find(|p| p.name() == provider)
                    .expect("the rule routes to one of the names it was given");
                Ok((routed, model))
            }
            ModelRoute::UnknownProvider { prefix } => {
                let truncated = truncate_prefix(prefix, MAX_PREFIX_IN_ERROR);
                Err(DomainError::Provider(format!(
                    "no configured provider '{}'; configured providers: {}. Switch the model \
                     to one of them as provider/model",
                    truncated,
                    names.join(", ")
                )))
            }
            ModelRoute::NoProviders => Err(DomainError::Provider(ERR_NO_PROVIDERS.to_string())),
        }
    }
}

impl ProviderRouter {
    /// Resolve the target provider and build a forwarding request with the
    /// provider prefix stripped.  All borrowed fields are forwarded as-is
    /// (zero-copy).
    fn forward<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> Result<(&'a Arc<dyn LlmProvider>, ChatRequest<'a>), DomainError> {
        let (provider, effective_model) = self.resolve(request.model)?;
        let req = ChatRequest {
            trace: request.trace,
            admission: request.admission,
            messages: request.messages,
            tools: request.tools,
            model: effective_model,
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            session_id: request.session_id,
            tool_choice: request.tool_choice,
            metadata: request.metadata,
            thinking_level: request.thinking_level,
            cancel_flag: request.cancel_flag,
            effort: request.effort,
        };
        Ok((provider, req))
    }
}

impl LlmProvider for ProviderRouter {
    fn route_check(&self, model: &str) -> crate::application::providers::ports::RouteCheck {
        use crate::application::providers::ports::RouteCheck;
        match route_model(model, &self.provider_names()) {
            ModelRoute::UnknownProvider { prefix } => RouteCheck::UnknownProvider {
                provider: truncate_prefix(prefix, MAX_PREFIX_IN_ERROR).to_string(),
                configured: self
                    .provider_names()
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
            },
            // A bare id goes to the first provider; none configured is
            // reported when a request is sent.
            ModelRoute::To { .. } | ModelRoute::NoProviders => RouteCheck::Routable,
        }
    }

    fn route_order(&self) -> Vec<String> {
        self.provider_names()
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    fn name(&self) -> &str {
        "router"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn chat<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        Box::pin(async move {
            let (provider, req) = self.forward(request)?;
            provider.chat(req).await
        })
    }

    fn chat_stream<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        Box::pin(async move {
            let (provider, req) = self.forward(request)?;
            provider.chat_stream(req).await
        })
    }

    fn chat_stream_incremental<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = tokio::sync::mpsc::Receiver<StreamEvent>> + Send + 'a>> {
        Box::pin(async move {
            let (provider, req) = match self.forward(request) {
                Ok(resolved) => resolved,
                Err(e) => {
                    let (tx, rx) = tokio::sync::mpsc::channel(1);
                    let _ = tx.send(StreamEvent::Error(e.to_string())).await;
                    return rx;
                }
            };
            provider.chat_stream_incremental(req).await
        })
    }
}

/// Maximum length of a provider prefix included in error messages.
const MAX_PREFIX_IN_ERROR: usize = 64;

/// Error when all providers are unavailable.
const ERR_NO_PROVIDERS: &str = "no LLM providers available";

/// Truncate a string to at most `max_bytes`, respecting UTF-8 char boundaries.
fn truncate_prefix(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
#[path = "router_tests.rs"]
mod tests;
