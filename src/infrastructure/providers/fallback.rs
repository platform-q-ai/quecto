// Fallback provider: wraps multiple LlmProviders with cooldown and error classification.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::domain::error::DomainError;
use crate::domain::message::LlmResponse;
use crate::domain::provider::{ChatRequest, LlmProvider};

use super::error::ErrorClass;

/// How long (in seconds) a provider is cooled down after a retryable error.
const DEFAULT_COOLDOWN_SECS: u64 = 60;

/// A provider entry with cooldown tracking.
#[derive(Debug)]
struct ProviderEntry {
    provider: Arc<dyn LlmProvider>,
    /// Unix timestamp (seconds) when the cooldown expires. 0 = no cooldown.
    cooldown_until: AtomicU64,
}

impl ProviderEntry {
    fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self {
            provider,
            cooldown_until: AtomicU64::new(0),
        }
    }

    fn is_available(&self) -> bool {
        let until = self.cooldown_until.load(Ordering::Relaxed);
        if until == 0 {
            return true;
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        now >= until
    }

    fn set_cooldown(&self, seconds: u64) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.cooldown_until.store(now + seconds, Ordering::Relaxed);
    }

    fn clear_cooldown(&self) {
        self.cooldown_until.store(0, Ordering::Relaxed);
    }
}

/// A provider that tries multiple underlying providers in order,
/// falling back to the next on retryable errors.
#[derive(Debug)]
pub struct FallbackProvider {
    entries: Vec<ProviderEntry>,
    cooldown_secs: u64,
}

impl FallbackProvider {
    /// Create a new fallback provider from an ordered list of providers.
    /// The first provider is preferred; subsequent ones are fallbacks.
    pub fn new(providers: Vec<Arc<dyn LlmProvider>>) -> Self {
        Self {
            entries: providers.into_iter().map(ProviderEntry::new).collect(),
            cooldown_secs: DEFAULT_COOLDOWN_SECS,
        }
    }

    /// Override the cooldown duration (for testing).
    pub fn with_cooldown_secs(mut self, secs: u64) -> Self {
        self.cooldown_secs = secs;
        self
    }

    /// Classify a DomainError into an ErrorClass for retry decisions.
    #[cfg(not(any(test, feature = "test-support")))]
    pub(crate) fn classify_error(err: &DomainError) -> ErrorClass {
        Self::classify_error_impl(err)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn classify_error(err: &DomainError) -> ErrorClass {
        Self::classify_error_impl(err)
    }

    fn classify_error_impl(err: &DomainError) -> ErrorClass {
        let msg = match err {
            DomainError::Provider(msg) => msg,
            _ => return ErrorClass::Unknown,
        };

        if let Some(status) = extract_http_status(msg) {
            return ErrorClass::from_status(status);
        }

        let lowered = msg.to_ascii_lowercase();

        if lowered.contains("request cancelled") || lowered.contains("request canceled") {
            return ErrorClass::Cancelled;
        }

        if lowered.contains("rate limit") {
            ErrorClass::RateLimit
        } else if lowered.contains("auth")
            || lowered.contains("unauthorized")
            || lowered.contains("forbidden")
            || lowered.contains("invalid api key")
            || lowered.contains("authentication")
        {
            ErrorClass::Auth
        } else if lowered.contains("internal server error")
            || lowered.contains("bad gateway")
            || lowered.contains("service unavailable")
            || lowered.contains("gateway timeout")
            || lowered.contains("overloaded_error")
        {
            ErrorClass::Server
        } else if lowered.contains("connect")
            || lowered.contains("connection")
            || lowered.contains("timeout")
            || lowered.contains("timed out")
            || lowered.contains("network")
            || lowered.contains("dns")
        {
            ErrorClass::Network
        } else {
            ErrorClass::Unknown
        }
    }

    /// Try to send a chat request, falling back through available providers.
    ///
    /// **Explicit routing via `provider/model` syntax**: when the model string
    /// contains a `/`, only providers whose name matches the prefix are tried,
    /// and the bare model id (the part after `/`) is forwarded. Unknown prefixes
    /// return an error immediately.
    ///
    /// **Bare model names** (no `/`) are tried against all available providers
    /// in insertion order; the first successful response wins.
    async fn try_chat(&self, request: &ChatRequest<'_>) -> Result<LlmResponse, DomainError> {
        let mut last_error: Option<DomainError> = None;
        let qualified = parse_qualified_model(request.model);
        let mut matched_qualified_provider = false;

        for entry in &self.entries {
            let effective_model = if let Some((provider_prefix, model_id)) = qualified {
                if !provider_prefix_matches(provider_prefix, entry.provider.name()) {
                    continue;
                }
                matched_qualified_provider = true;
                model_id
            } else {
                request.model
            };

            if !entry.is_available() {
                continue;
            }

            let req = ChatRequest {
                messages: request.messages,
                tools: request.tools,
                model: effective_model,
                max_tokens: request.max_tokens,
                temperature: request.temperature,
                session_id: request.session_id.clone(),
                tool_choice: request.tool_choice.clone(),
                metadata: request.metadata.clone(),
                thinking_level: request.thinking_level,
                cancel_flag: request.cancel_flag.clone(),
            };
            match entry.provider.chat(req).await {
                Ok(response) => {
                    entry.clear_cooldown();
                    return Ok(response);
                }
                Err(err) => {
                    let class = Self::classify_error(&err);
                    if class.is_retryable() {
                        entry.set_cooldown(self.cooldown_secs);
                    }
                    last_error = Some(err);
                    // If retryable, continue to next provider
                    if !class.is_retryable() {
                        // Non-retryable errors should not fallback
                        return Err(last_error.unwrap());
                    }
                }
            }
        }

        if let Some((provider_prefix, _)) = qualified {
            if !matched_qualified_provider {
                let truncated = &provider_prefix[..provider_prefix.len().min(MAX_PREFIX_IN_ERROR)];
                return Err(DomainError::Provider(format!(
                    "no configured provider matches model prefix '{}'",
                    truncated
                )));
            }
        }

        Err(last_error
            .unwrap_or_else(|| DomainError::Provider("no providers available".to_string())))
    }
}

/// Parse a `provider/model` string into its two parts.
///
/// Returns `None` for bare model names (no `/`) or malformed inputs.
/// Rejects nested slashes in the model segment (`a/b/c`) to avoid
/// forwarding multi-segment paths like `models/gpt-4o` to providers.
fn parse_qualified_model(model: &str) -> Option<(&str, &str)> {
    let (provider, model_id) = model.split_once('/')?;
    let provider = provider.trim();
    let model_id = model_id.trim();
    // Reject empty segments or nested slashes in the model id.
    if provider.is_empty() || model_id.is_empty() || model_id.contains('/') {
        return None;
    }
    Some((provider, model_id))
}

/// Returns `true` when `prefix` names the same provider as `provider_name`.
///
/// Supports well-known aliases:
/// - `"openai"` and `"openai-codex"` both resolve to the `"codex"` provider
///   (ChatGPT OAuth token path).
///
/// Comparison is case-insensitive. Provider names from registered providers
/// are expected to be trimmed; the prefix comes from user input and is also
/// trimmed by `parse_qualified_model` before reaching here.
fn provider_prefix_matches(prefix: &str, provider_name: &str) -> bool {
    if prefix.eq_ignore_ascii_case(provider_name) {
        return true;
    }

    // "openai" and "openai-codex" are conventional aliases for the Codex
    // provider (ChatGPT backend using OAuth tokens).
    if provider_name.eq_ignore_ascii_case("codex") {
        return prefix.eq_ignore_ascii_case("openai")
            || prefix.eq_ignore_ascii_case("openai-codex");
    }

    false
}

/// Maximum length of a provider prefix included in error messages.
/// Prevents unbounded user input from bloating log lines.
const MAX_PREFIX_IN_ERROR: usize = 64;

fn extract_http_status(msg: &str) -> Option<u16> {
    let lowered = msg.to_ascii_lowercase();

    for marker in ["http", "status", "code"] {
        let mut search_from = 0;
        while let Some(rel) = lowered[search_from..].find(marker) {
            let idx = search_from + rel + marker.len();
            if let Some(code) = parse_status_near(&lowered[idx..]) {
                return Some(code);
            }
            search_from = idx;
        }
    }

    None
}

fn parse_status_near(s: &str) -> Option<u16> {
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_digit() {
            if i + 2 >= bytes.len()
                || !bytes[i + 1].is_ascii_digit()
                || !bytes[i + 2].is_ascii_digit()
            {
                return None;
            }
            let code = ((bytes[i] - b'0') as u16) * 100
                + ((bytes[i + 1] - b'0') as u16) * 10
                + ((bytes[i + 2] - b'0') as u16);
            return (100..=599).contains(&code).then_some(code);
        }

        if !(b.is_ascii_whitespace() || b == b':' || b == b'=' || b == b'-' || b == b'/') {
            return None;
        }
        i += 1;
    }

    None
}

impl LlmProvider for FallbackProvider {
    fn name(&self) -> &str {
        "fallback"
    }

    fn chat(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        // Clone references into owned data so the future only borrows &self.
        let messages = request.messages.to_vec();
        let tools = request.tools.to_vec();
        let model = request.model.to_string();
        Box::pin(async move {
            let req = ChatRequest {
                messages: &messages,
                tools: &tools,
                model: &model,
                max_tokens: request.max_tokens,
                temperature: request.temperature,
                session_id: request.session_id.clone(),
                tool_choice: request.tool_choice.clone(),
                metadata: request.metadata.clone(),
                thinking_level: request.thinking_level,
                cancel_flag: request.cancel_flag.clone(),
            };
            self.try_chat(&req).await
        })
    }
}

#[cfg(test)]
#[path = "fallback_tests.rs"]
mod tests;
