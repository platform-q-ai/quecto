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
        thinking_level: None,
        cancel_flag: None,
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
fn test_classify_cancelled_error() {
    let err = DomainError::Provider("request cancelled".to_string());
    assert_eq!(
        FallbackProvider::classify_error(&err),
        ErrorClass::Cancelled
    );
    assert!(!ErrorClass::Cancelled.is_retryable());
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

    fn received_model(&self) -> Option<String> {
        self.received_model.lock().unwrap().clone()
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
        thinking_level: None,
        cancel_flag: None,
    }
}

// ─── Bare model names (no provider prefix) ──────────────────────────────

#[tokio::test]
async fn test_bare_model_goes_to_first_provider_in_order() {
    // Without provider/ prefix, bare model names are tried against
    // providers in insertion order (no implicit model-name routing).
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

    // Goes to openai (first in list) — no smart matching.
    assert_eq!(resp.content.unwrap(), "OpenAI response");
    assert!(openai.was_called());
    assert!(!anthropic.was_called());
}

#[tokio::test]
async fn test_bare_gpt_model_goes_to_first_provider() {
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
    assert!(openai.was_called());
    assert!(!anthropic.was_called());
}

#[tokio::test]
async fn test_bare_model_falls_back_on_retryable_error() {
    let openai = TrackingProvider::failing("openai", "HTTP 500 Internal Server Error");
    let anthropic = TrackingProvider::succeeding("anthropic", "Fallback response");
    let provider = FallbackProvider::new(vec![
        openai.clone() as Arc<dyn LlmProvider>,
        anthropic.clone() as Arc<dyn LlmProvider>,
    ]);

    let messages = test_messages();
    let resp = provider
        .chat(make_request(&messages, "claude-sonnet-4-20250514"))
        .await
        .unwrap();

    assert_eq!(resp.content.unwrap(), "Fallback response");
    assert!(openai.was_called());
    assert!(anthropic.was_called());
}

// ─── Explicit provider/model routing ──────────────────────────────────────

#[tokio::test]
async fn test_explicit_anthropic_prefix_routes_to_anthropic() {
    let openai = TrackingProvider::succeeding("openai", "OpenAI response");
    let anthropic = TrackingProvider::succeeding("anthropic", "Anthropic response");
    let provider = FallbackProvider::new(vec![
        openai.clone() as Arc<dyn LlmProvider>,
        anthropic.clone() as Arc<dyn LlmProvider>,
    ]);

    let messages = test_messages();
    let resp = provider
        .chat(make_request(&messages, "anthropic/claude-opus-4-5"))
        .await
        .unwrap();

    assert_eq!(resp.content.unwrap(), "Anthropic response");
    assert_eq!(
        anthropic.received_model().as_deref(),
        Some("claude-opus-4-5")
    );
    assert!(!openai.was_called());
}

#[tokio::test]
async fn test_explicit_anthropic_prefix_with_no_anthropic_provider_fails() {
    let openai = TrackingProvider::succeeding("openai", "OpenAI response");
    let provider = FallbackProvider::new(vec![openai.clone() as Arc<dyn LlmProvider>]);

    let messages = test_messages();
    let result = provider
        .chat(make_request(&messages, "anthropic/claude-opus-4-5"))
        .await;

    let err = result.expect_err("should fail when no anthropic provider");
    assert!(err.to_string().contains("no configured provider matches"));
    assert!(!openai.was_called());
}

#[tokio::test]
async fn test_provider_qualified_model_routes_and_strips_prefix() {
    let openai = TrackingProvider::succeeding("openai", "OpenAI response");
    let anthropic = TrackingProvider::succeeding("anthropic", "Anthropic response");
    let provider = FallbackProvider::new(vec![
        openai.clone() as Arc<dyn LlmProvider>,
        anthropic.clone() as Arc<dyn LlmProvider>,
    ]);

    let messages = test_messages();
    let resp = provider
        .chat(make_request(&messages, "openai/gpt-4o"))
        .await
        .unwrap();

    assert_eq!(resp.content.unwrap(), "OpenAI response");
    assert_eq!(openai.received_model().as_deref(), Some("gpt-4o"));
    assert!(!anthropic.was_called());
}

#[tokio::test]
async fn test_provider_qualified_openai_prefix_matches_codex_provider() {
    let codex = TrackingProvider::succeeding("codex", "Codex response");
    let anthropic = TrackingProvider::succeeding("anthropic", "Anthropic response");
    let provider = FallbackProvider::new(vec![
        codex.clone() as Arc<dyn LlmProvider>,
        anthropic.clone() as Arc<dyn LlmProvider>,
    ]);

    let messages = test_messages();
    let resp = provider
        .chat(make_request(&messages, "openai/gpt-5.3-codex"))
        .await
        .unwrap();

    assert_eq!(resp.content.unwrap(), "Codex response");
    assert_eq!(codex.received_model().as_deref(), Some("gpt-5.3-codex"));
    assert!(!anthropic.was_called());
}

#[tokio::test]
async fn test_provider_qualified_model_with_unknown_prefix_fails_fast() {
    let openai = TrackingProvider::succeeding("openai", "OpenAI response");
    let provider = FallbackProvider::new(vec![openai.clone() as Arc<dyn LlmProvider>]);

    let messages = test_messages();
    let result = provider
        .chat(make_request(&messages, "unknown/gpt-4o"))
        .await;

    let err = result.expect_err("unknown provider prefix should fail");
    assert!(
        err.to_string()
            .contains("no configured provider matches model prefix"),
        "unexpected error: {err}"
    );
    assert!(!openai.was_called());
}

#[tokio::test]
async fn test_nested_slash_model_treated_as_bare_name() {
    // "openai/models/gpt-4o" has a nested slash in the model segment.
    // parse_qualified_model rejects it, so it falls through as a bare name.
    let openai = TrackingProvider::succeeding("openai", "OpenAI response");
    let provider = FallbackProvider::new(vec![openai.clone() as Arc<dyn LlmProvider>]);

    let messages = test_messages();
    let resp = provider
        .chat(make_request(&messages, "openai/models/gpt-4o"))
        .await
        .unwrap();

    // Treated as bare name — first provider gets the full string unchanged.
    assert_eq!(resp.content.unwrap(), "OpenAI response");
    assert_eq!(
        openai.received_model().as_deref(),
        Some("openai/models/gpt-4o")
    );
}
