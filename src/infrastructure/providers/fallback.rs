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
    fn classify_error(err: &DomainError) -> ErrorClass {
        let msg = err.to_string();
        // Try to extract an HTTP status code from the error message
        if msg.contains("429") || msg.to_lowercase().contains("rate limit") {
            ErrorClass::RateLimit
        } else if msg.contains("401") || msg.contains("403") || msg.to_lowercase().contains("auth")
        {
            ErrorClass::Auth
        } else if msg.contains("500")
            || msg.contains("502")
            || msg.contains("503")
            || msg.contains("504")
        {
            ErrorClass::Server
        } else if msg.to_lowercase().contains("connect")
            || msg.to_lowercase().contains("timeout")
            || msg.to_lowercase().contains("network")
        {
            ErrorClass::Network
        } else {
            ErrorClass::Unknown
        }
    }

    /// Try to send a chat request, falling back through available providers.
    async fn try_chat(&self, request: &ChatRequest<'_>) -> Result<LlmResponse, DomainError> {
        let mut last_error: Option<DomainError> = None;

        for entry in &self.entries {
            if !entry.is_available() {
                continue;
            }

            let req = ChatRequest {
                messages: request.messages,
                tools: request.tools,
                model: request.model,
                max_tokens: request.max_tokens,
                temperature: request.temperature,
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
            };
            self.try_chat(&req).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::message::{Message, Role};
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
        vec![Message {
            role: Role::User,
            content: "Hi".to_string(),
            tool_calls: vec![],
            tool_call_id: None,
        }]
    }

    fn test_request(messages: &[Message]) -> ChatRequest<'_> {
        ChatRequest {
            messages,
            tools: &[],
            model: "gpt-4",
            max_tokens: 1024,
            temperature: 0.7,
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
}
