// Fallback provider: wraps multiple LlmProviders with cooldown and error classification.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::domain::error::DomainError;
use crate::domain::message::LlmResponse;
use crate::domain::provider::{ChatRequest, LlmProvider, model_excluded_from_provider};

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
    fn classify_error(err: &DomainError) -> ErrorClass {
        let msg = match err {
            DomainError::Provider(msg) => msg,
            _ => return ErrorClass::Unknown,
        };

        if let Some(status) = extract_http_status(msg) {
            return ErrorClass::from_status(status);
        }

        let lowered = msg.to_ascii_lowercase();

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
    /// Model-aware routing: if the model name has a definitive owner (e.g.
    /// `claude-*` → Anthropic), only that provider is tried. Unknown model
    /// names fall through all providers in insertion order as before.
    ///
    /// See [`model_excluded_from_provider`] in `domain::provider` for routing rules.
    async fn try_chat(&self, request: &ChatRequest<'_>) -> Result<LlmResponse, DomainError> {
        let mut last_error: Option<DomainError> = None;

        for entry in &self.entries {
            // Skip providers that definitely cannot serve this model.
            if model_excluded_from_provider(request.model, entry.provider.name()) {
                continue;
            }

            if !entry.is_available() {
                continue;
            }

            let req = ChatRequest {
                messages: request.messages,
                tools: request.tools,
                model: request.model,
                max_tokens: request.max_tokens,
                temperature: request.temperature,
                session_id: request.session_id.clone(),
                tool_choice: request.tool_choice.clone(),
                metadata: request.metadata.clone(),
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

        Err(last_error
            .unwrap_or_else(|| DomainError::Provider("no providers available".to_string())))
    }
}

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
            };
            self.try_chat(&req).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::message::Message;
    use std::sync::Mutex;

    /// Test provider that either succeeds or fails with a configurable error.
    #[derive(Debug)]
    struct TestProvider {
        name: String,
        response: Mutex<Result<LlmResponse, String>>,
    }

    impl TestProvider {
        fn succeeding(name: &str, content: &str) -> Arc<Self> {
            Arc::new(Self {
                name: name.to_string(),
                response: Mutex::new(Ok(LlmResponse {
                    content: Some(content.to_string()),
                    tool_calls: vec![],
                    usage: None,
                    stop_reason: None,
                })),
            })
        }

        fn failing(name: &str, error: &str) -> Arc<Self> {
            Arc::new(Self {
                name: name.to_string(),
                response: Mutex::new(Err(error.to_string())),
            })
        }
    }

    impl LlmProvider for TestProvider {
        fn name(&self) -> &str {
            &self.name
        }

        fn chat(
            &self,
            _request: ChatRequest<'_>,
        ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
            let result = self.response.lock().unwrap().clone();
            Box::pin(async move {
                match result {
                    Ok(r) => Ok(r),
                    Err(e) => Err(DomainError::Provider(e)),
                }
            })
        }
    }

    fn test_messages() -> Vec<Message> {
        vec![Message::user("Hi")]
    }

    fn test_request(messages: &[Message]) -> ChatRequest<'_> {
        ChatRequest {
            messages,
            tools: &[],
            model: "gpt-4",
            max_tokens: 1024,
            temperature: 0.7,
            session_id: None,
            tool_choice: None,
            metadata: None,
        }
    }

    #[tokio::test]
    async fn test_primary_succeeds() {
        let primary = TestProvider::succeeding("openai", "Hello!");
        let fallback_prov = TestProvider::succeeding("anthropic", "Fallback hello");
        let provider = FallbackProvider::new(vec![primary as Arc<dyn LlmProvider>, fallback_prov]);

        let resp = provider.chat(test_request(&test_messages())).await.unwrap();
        assert_eq!(resp.content.unwrap(), "Hello!");
    }

    #[tokio::test]
    async fn test_fallback_on_server_error() {
        let primary = TestProvider::failing("openai", "HTTP 500 Internal Server Error");
        let fallback_prov = TestProvider::succeeding("anthropic", "Fallback response");
        let provider = FallbackProvider::new(vec![primary as Arc<dyn LlmProvider>, fallback_prov]);

        let resp = provider.chat(test_request(&test_messages())).await.unwrap();
        assert_eq!(resp.content.unwrap(), "Fallback response");
    }

    #[tokio::test]
    async fn test_fallback_on_rate_limit() {
        let primary = TestProvider::failing("openai", "HTTP 429 rate limit exceeded");
        let fallback_prov = TestProvider::succeeding("anthropic", "Fallback OK");
        let provider = FallbackProvider::new(vec![primary as Arc<dyn LlmProvider>, fallback_prov]);

        let resp = provider.chat(test_request(&test_messages())).await.unwrap();
        assert_eq!(resp.content.unwrap(), "Fallback OK");
    }

    #[tokio::test]
    async fn test_no_fallback_on_auth_error() {
        let primary = TestProvider::failing("openai", "HTTP 401 Unauthorized");
        let fallback_prov = TestProvider::succeeding("anthropic", "Should not reach");
        let provider = FallbackProvider::new(vec![primary as Arc<dyn LlmProvider>, fallback_prov]);

        let result = provider.chat(test_request(&test_messages())).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("401"));
    }

    #[tokio::test]
    async fn test_cooldown_skips_provider() {
        let primary = TestProvider::failing("openai", "HTTP 500 server error");
        let fallback_prov = TestProvider::succeeding("anthropic", "Fallback");
        let provider = FallbackProvider::new(vec![primary as Arc<dyn LlmProvider>, fallback_prov])
            .with_cooldown_secs(60);

        // First call: primary fails, falls back to anthropic
        let resp1 = provider.chat(test_request(&test_messages())).await.unwrap();
        assert_eq!(resp1.content.unwrap(), "Fallback");

        // Second call: primary should be on cooldown, goes straight to anthropic
        let resp2 = provider.chat(test_request(&test_messages())).await.unwrap();
        assert_eq!(resp2.content.unwrap(), "Fallback");
    }

    #[tokio::test]
    async fn test_all_providers_fail() {
        let p1 = TestProvider::failing("openai", "HTTP 500 error");
        let p2 = TestProvider::failing("anthropic", "HTTP 503 unavailable");
        let provider = FallbackProvider::new(vec![p1 as Arc<dyn LlmProvider>, p2]);

        let result = provider.chat(test_request(&test_messages())).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_no_providers() {
        let provider = FallbackProvider::new(vec![]);
        let result = provider.chat(test_request(&test_messages())).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no providers"));
    }

    #[test]
    fn test_classify_rate_limit() {
        let err = DomainError::Provider("HTTP 429 rate limit".to_string());
        assert_eq!(
            FallbackProvider::classify_error(&err),
            ErrorClass::RateLimit
        );
    }

    #[test]
    fn test_classify_server_error() {
        let err = DomainError::Provider("HTTP 500 Internal Server Error".to_string());
        assert_eq!(FallbackProvider::classify_error(&err), ErrorClass::Server);
    }

    #[test]
    fn test_classify_auth_error() {
        let err = DomainError::Provider("HTTP 401 Unauthorized".to_string());
        assert_eq!(FallbackProvider::classify_error(&err), ErrorClass::Auth);
    }

    #[test]
    fn test_classify_network_error() {
        let err = DomainError::Provider("connection timeout".to_string());
        assert_eq!(FallbackProvider::classify_error(&err), ErrorClass::Network);

        let err2 = DomainError::Provider("network unreachable".to_string());
        assert_eq!(FallbackProvider::classify_error(&err2), ErrorClass::Network);

        let err3 = DomainError::Provider("connect refused".to_string());
        assert_eq!(FallbackProvider::classify_error(&err3), ErrorClass::Network);
    }

    #[test]
    fn test_classify_unknown_error() {
        let err = DomainError::Provider("something unexpected happened".to_string());
        assert_eq!(FallbackProvider::classify_error(&err), ErrorClass::Unknown);
    }

    #[test]
    fn test_classify_403_as_auth() {
        let err = DomainError::Provider("HTTP 403 Forbidden".to_string());
        assert_eq!(FallbackProvider::classify_error(&err), ErrorClass::Auth);
    }

    #[test]
    fn test_classify_502_503_504() {
        for code in ["502", "503", "504"] {
            let err = DomainError::Provider(format!("HTTP {} Bad Gateway", code));
            assert_eq!(
                FallbackProvider::classify_error(&err),
                ErrorClass::Server,
                "expected Server for {}",
                code
            );
        }
    }

    #[test]
    fn test_non_provider_errors_are_not_classified_as_provider_server_errors() {
        let err = DomainError::Tool("HTTP 500 from subprocess".to_string());
        assert_eq!(FallbackProvider::classify_error(&err), ErrorClass::Unknown);
    }

    #[test]
    fn test_classify_auth_by_semantic_message() {
        let err = DomainError::Provider("Authentication failed: invalid api key".to_string());
        assert_eq!(FallbackProvider::classify_error(&err), ErrorClass::Auth);
    }

    #[test]
    fn test_status_extraction_prefers_http_context_over_other_numbers() {
        let err = DomainError::Provider(
            "connect to 10.0.0.1:443 failed, HTTP 503 Service Unavailable".to_string(),
        );
        assert_eq!(FallbackProvider::classify_error(&err), ErrorClass::Server);
    }

    #[test]
    fn test_fallback_provider_name() {
        let provider = FallbackProvider::new(vec![]);
        assert_eq!(provider.name(), "fallback");
    }

    #[test]
    fn test_with_cooldown_secs() {
        let provider = FallbackProvider::new(vec![]).with_cooldown_secs(120);
        assert_eq!(provider.cooldown_secs, 120);
    }

    // ── Model routing tests ────────────────────────────────────────────────────

    /// A TestProvider that also records the model name it received.
    #[derive(Debug)]
    struct TrackingProvider {
        name: String,
        response: Mutex<Result<LlmResponse, String>>,
        received_model: Mutex<Option<String>>,
    }

    impl TrackingProvider {
        fn succeeding(name: &str, content: &str) -> Arc<Self> {
            Arc::new(Self {
                name: name.to_string(),
                response: Mutex::new(Ok(LlmResponse {
                    content: Some(content.to_string()),
                    tool_calls: vec![],
                    usage: None,
                    stop_reason: None,
                })),
                received_model: Mutex::new(None),
            })
        }

        fn failing(name: &str, error: &str) -> Arc<Self> {
            Arc::new(Self {
                name: name.to_string(),
                response: Mutex::new(Err(error.to_string())),
                received_model: Mutex::new(None),
            })
        }

        fn was_called(&self) -> bool {
            self.received_model.lock().unwrap().is_some()
        }
    }

    impl LlmProvider for TrackingProvider {
        fn name(&self) -> &str {
            &self.name
        }

        fn chat(
            &self,
            request: ChatRequest<'_>,
        ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
            *self.received_model.lock().unwrap() = Some(request.model.to_string());
            let result = self.response.lock().unwrap().clone();
            Box::pin(async move {
                match result {
                    Ok(r) => Ok(r),
                    Err(e) => Err(DomainError::Provider(e)),
                }
            })
        }
    }

    fn make_request<'a>(messages: &'a [Message], model: &'a str) -> ChatRequest<'a> {
        ChatRequest {
            messages,
            tools: &[],
            model,
            max_tokens: 1024,
            temperature: 0.7,
            session_id: None,
            tool_choice: None,
            metadata: None,
        }
    }

    #[tokio::test]
    async fn test_claude_model_routes_to_anthropic_not_openai() {
        let openai = TrackingProvider::succeeding("openai", "OpenAI response");
        let anthropic = TrackingProvider::succeeding("anthropic", "Anthropic response");
        let provider = FallbackProvider::new(vec![
            openai.clone() as Arc<dyn LlmProvider>,
            anthropic.clone() as Arc<dyn LlmProvider>,
        ]);

        let messages = test_messages();
        let resp = provider
            .chat(make_request(&messages, "claude-opus-4-5"))
            .await
            .unwrap();

        assert_eq!(resp.content.unwrap(), "Anthropic response");
        assert!(
            !openai.was_called(),
            "OpenAI should NOT be called for claude-* models"
        );
        assert!(
            anthropic.was_called(),
            "Anthropic should be called for claude-* models"
        );
    }

    #[tokio::test]
    async fn test_gpt_model_routes_to_openai() {
        let openai = TrackingProvider::succeeding("openai", "OpenAI response");
        let anthropic = TrackingProvider::succeeding("anthropic", "Anthropic response");
        let provider = FallbackProvider::new(vec![
            openai.clone() as Arc<dyn LlmProvider>,
            anthropic.clone() as Arc<dyn LlmProvider>,
        ]);

        let messages = test_messages();
        let resp = provider
            .chat(make_request(&messages, "gpt-4o"))
            .await
            .unwrap();

        assert_eq!(resp.content.unwrap(), "OpenAI response");
        assert!(
            openai.was_called(),
            "OpenAI should be called for gpt-* models"
        );
        assert!(
            !anthropic.was_called(),
            "Anthropic should NOT be called for gpt-* models"
        );
    }

    #[tokio::test]
    async fn test_claude_model_bypasses_failing_openai() {
        let openai = TrackingProvider::failing("openai", "HTTP 500 Internal Server Error");
        let anthropic = TrackingProvider::succeeding("anthropic", "Claude response");
        let provider = FallbackProvider::new(vec![
            openai.clone() as Arc<dyn LlmProvider>,
            anthropic.clone() as Arc<dyn LlmProvider>,
        ]);

        let messages = test_messages();
        let resp = provider
            .chat(make_request(&messages, "claude-sonnet-4-20250514"))
            .await
            .unwrap();

        assert_eq!(resp.content.unwrap(), "Claude response");
        assert!(
            !openai.was_called(),
            "OpenAI should NOT be called for claude-* models"
        );
    }

    #[tokio::test]
    async fn test_unknown_model_falls_through_in_order() {
        let openai = TrackingProvider::succeeding("openai", "OpenAI response");
        let anthropic = TrackingProvider::succeeding("anthropic", "Anthropic response");
        let provider = FallbackProvider::new(vec![
            openai.clone() as Arc<dyn LlmProvider>,
            anthropic.clone() as Arc<dyn LlmProvider>,
        ]);

        let messages = test_messages();
        let resp = provider
            .chat(make_request(&messages, "some-unknown-model"))
            .await
            .unwrap();

        assert_eq!(resp.content.unwrap(), "OpenAI response");
        assert!(openai.was_called());
        assert!(!anthropic.was_called());
    }

    #[tokio::test]
    async fn test_claude_model_with_no_anthropic_provider_fails() {
        let openai = TrackingProvider::succeeding("openai", "OpenAI response");
        let provider = FallbackProvider::new(vec![openai.clone() as Arc<dyn LlmProvider>]);

        let messages = test_messages();
        let result = provider
            .chat(make_request(&messages, "claude-opus-4-5"))
            .await;

        assert!(
            result.is_err(),
            "should fail when no anthropic provider available"
        );
        assert!(
            !openai.was_called(),
            "OpenAI should NOT be called for claude-* models"
        );
    }

    #[tokio::test]
    async fn test_claude_model_routing_is_case_insensitive() {
        // Model names are typically lowercase but verify robustness
        let openai = TrackingProvider::succeeding("openai", "OpenAI response");
        let anthropic = TrackingProvider::succeeding("anthropic", "Anthropic response");
        let provider = FallbackProvider::new(vec![
            openai.clone() as Arc<dyn LlmProvider>,
            anthropic.clone() as Arc<dyn LlmProvider>,
        ]);

        let messages = test_messages();
        let resp = provider
            .chat(make_request(&messages, "claude-3-5-sonnet"))
            .await
            .unwrap();

        assert_eq!(resp.content.unwrap(), "Anthropic response");
        assert!(!openai.was_called());
        assert!(anthropic.was_called());
    }
}
