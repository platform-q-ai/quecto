use super::*;
use crate::domain::message::Message;
use std::sync::Mutex;

/// Test provider that either succeeds or fails.
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

#[tokio::test]
async fn test_single_provider_succeeds() {
    let primary = TestProvider::succeeding("openai", "Hello!");
    let router = ProviderRouter::new(vec![primary as Arc<dyn LlmProvider>]);

    let resp = router.chat(test_request(&test_messages())).await.unwrap();
    assert_eq!(resp.content.unwrap(), "Hello!");
}

#[tokio::test]
async fn test_no_fallback_on_server_error() {
    let primary = TestProvider::failing("openai", "HTTP 500 Internal Server Error");
    let secondary = TestProvider::succeeding("anthropic", "Should not reach");
    let router = ProviderRouter::new(vec![primary as Arc<dyn LlmProvider>, secondary]);

    // Bare model → first provider (openai), which fails. No fallback.
    let result = router.chat(test_request(&test_messages())).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("500"));
}

#[tokio::test]
async fn test_no_providers() {
    let router = ProviderRouter::new(vec![]);
    let result = router.chat(test_request(&test_messages())).await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("no LLM providers available")
    );
}

#[tokio::test]
async fn test_router_name() {
    let router = ProviderRouter::new(vec![]);
    assert_eq!(router.name(), "router");
}

// ── Model routing ──────────────────────────────────────────────────────

/// Provider that records the model name it received.
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

#[tokio::test]
async fn test_explicit_prefix_routes_to_correct_provider() {
    let openai = TrackingProvider::succeeding("openai", "OpenAI");
    let anthropic = TrackingProvider::succeeding("anthropic", "Anthropic");
    let router = ProviderRouter::new(vec![
        openai.clone() as Arc<dyn LlmProvider>,
        anthropic.clone(),
    ]);

    let messages = test_messages();
    let resp = router
        .chat(make_request(&messages, "anthropic/claude-opus-4-5"))
        .await
        .unwrap();
    assert_eq!(resp.content.unwrap(), "Anthropic");
    assert_eq!(
        anthropic.received_model().as_deref(),
        Some("claude-opus-4-5")
    );
    assert!(!openai.was_called());
}

#[tokio::test]
async fn test_bare_model_goes_to_first_provider() {
    let openai = TrackingProvider::succeeding("openai", "OpenAI");
    let anthropic = TrackingProvider::succeeding("anthropic", "Anthropic");
    let router = ProviderRouter::new(vec![
        openai.clone() as Arc<dyn LlmProvider>,
        anthropic.clone(),
    ]);

    let messages = test_messages();
    let _ = router
        .chat(make_request(&messages, "gpt-4o"))
        .await
        .unwrap();
    assert!(openai.was_called());
    assert!(!anthropic.was_called());
}

#[tokio::test]
async fn test_unknown_prefix_fails_fast() {
    let openai = TrackingProvider::succeeding("openai", "OpenAI");
    let router = ProviderRouter::new(vec![openai.clone() as Arc<dyn LlmProvider>]);

    let messages = test_messages();
    let result = router.chat(make_request(&messages, "unknown/gpt-4o")).await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("no configured provider")
    );
    assert!(!openai.was_called());
}

#[tokio::test]
async fn test_openai_prefix_matches_codex_provider() {
    let codex = TrackingProvider::succeeding("codex", "Codex");
    let router = ProviderRouter::new(vec![codex.clone() as Arc<dyn LlmProvider>]);

    let messages = test_messages();
    let resp = router
        .chat(make_request(&messages, "openai/gpt-4o"))
        .await
        .unwrap();
    assert_eq!(resp.content.unwrap(), "Codex");
    assert_eq!(codex.received_model().as_deref(), Some("gpt-4o"));
}

#[tokio::test]
async fn test_nested_slash_treated_as_bare_name() {
    let openai = TrackingProvider::succeeding("openai", "OpenAI");
    let router = ProviderRouter::new(vec![openai.clone() as Arc<dyn LlmProvider>]);

    let messages = test_messages();
    let _ = router
        .chat(make_request(&messages, "openai/models/gpt-4o"))
        .await
        .unwrap();
    assert_eq!(
        openai.received_model().as_deref(),
        Some("openai/models/gpt-4o")
    );
}

// ── Zero-copy forwarding (#370) ────────────────────────────────────────

/// Provider that captures the pointer address of the messages slice.
#[derive(Debug)]
struct SlicePtrProvider {
    captured_msg_ptr: Mutex<Option<usize>>,
    captured_tools_ptr: Mutex<Option<usize>>,
}

impl LlmProvider for SlicePtrProvider {
    fn name(&self) -> &str {
        "test"
    }

    fn chat(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        *self.captured_msg_ptr.lock().unwrap() = Some(request.messages.as_ptr() as usize);
        *self.captured_tools_ptr.lock().unwrap() = Some(request.tools.as_ptr() as usize);
        Box::pin(async move {
            Ok(LlmResponse {
                content: Some("ok".to_string()),
                tool_calls: vec![],
                usage: None,
                stop_reason: None,
            })
        })
    }
}

#[tokio::test]
async fn test_chat_forwards_messages_without_cloning() {
    let inner = Arc::new(SlicePtrProvider {
        captured_msg_ptr: Mutex::new(None),
        captured_tools_ptr: Mutex::new(None),
    });
    let router = ProviderRouter::new(vec![inner.clone() as Arc<dyn LlmProvider>]);

    let messages = test_messages();
    let original_ptr = messages.as_ptr() as usize;

    let _ = router.chat(test_request(&messages)).await.unwrap();

    let forwarded_ptr = inner.captured_msg_ptr.lock().unwrap().unwrap();
    assert_eq!(
        original_ptr, forwarded_ptr,
        "ProviderRouter should forward the original messages slice without cloning",
    );
}

#[tokio::test]
async fn test_chat_forwards_tools_without_cloning() {
    let inner = Arc::new(SlicePtrProvider {
        captured_msg_ptr: Mutex::new(None),
        captured_tools_ptr: Mutex::new(None),
    });
    let router = ProviderRouter::new(vec![inner.clone() as Arc<dyn LlmProvider>]);

    let messages = test_messages();
    let tools = vec![crate::domain::tool::ToolDefinition {
        name: "bash".into(),
        description: "run commands".into(),
        parameters_schema: "{}".into(),
    }];
    let request = ChatRequest {
        messages: &messages,
        tools: &tools,
        model: "gpt-4",
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
    };
    let original_ptr = tools.as_ptr() as usize;

    let _ = router.chat(request).await.unwrap();

    let forwarded_ptr = inner.captured_tools_ptr.lock().unwrap().unwrap();
    assert_eq!(
        original_ptr, forwarded_ptr,
        "ProviderRouter should forward the original tools slice without cloning",
    );
}
