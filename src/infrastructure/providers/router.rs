// Provider router: routes ChatRequests to the correct provider based on
// `provider/model-id` syntax, where model-id is opaque. No fallback, no
// cloning, no cooldown.
//
// Replaces FallbackProvider (#370).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::domain::error::DomainError;
use crate::domain::message::LlmResponse;
use crate::domain::provider::{ChatRequest, LlmProvider, StreamEvent};

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
        if let Some((prefix, bare_model)) = parse_qualified_model(model) {
            for p in &self.providers {
                if provider_prefix_matches(prefix, p.name()) {
                    return Ok((p, bare_model));
                }
            }
            let truncated = truncate_prefix(prefix, MAX_PREFIX_IN_ERROR);
            return Err(DomainError::Provider(format!(
                "no configured provider '{}'",
                truncated
            )));
        }

        // Bare model → first provider
        self.providers
            .first()
            .map(|p| (p, model))
            .ok_or_else(|| DomainError::Provider(ERR_NO_PROVIDERS.to_string()))
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
    fn name(&self) -> &str {
        "router"
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

/// Parse a UI/CLI `provider/model-id` string into provider and model id.
///
/// Returns `None` for bare model names (no `/`) or malformed inputs. The split
/// happens exactly once at the first slash: the provider is a routing key, while
/// the model id is opaque and may itself contain slashes (for example Fireworks
/// serverless ids like `accounts/fireworks/models/glm-5p2`).
fn parse_qualified_model(model: &str) -> Option<(&str, &str)> {
    let (provider, model_id) = model.split_once('/')?;
    let provider = provider.trim();
    let model_id = model_id.trim();
    if provider.is_empty() || model_id.is_empty() {
        return None;
    }
    Some((provider, model_id))
}

/// Returns `true` when `prefix` names the same provider as `provider_name`.
///
/// Supports well-known aliases:
/// - `"openai"` and `"openai-codex"` both resolve to the `"codex"` provider
///   (ChatGPT OAuth token path).
fn provider_prefix_matches(prefix: &str, provider_name: &str) -> bool {
    if prefix.eq_ignore_ascii_case(provider_name) {
        return true;
    }
    if provider_name.eq_ignore_ascii_case("codex") {
        return prefix.eq_ignore_ascii_case("openai")
            || prefix.eq_ignore_ascii_case("openai-codex");
    }
    false
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
