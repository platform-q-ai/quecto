use super::*;

// Provider Steps
// ===========================================================================

#[given(expr = "a config with provider {string} and api_key {string}")]
fn given_provider_config(world: &mut QuectoWorld, provider_name: String, api_key: String) {
    world.provider = providers::create_provider(&provider_name, api_key, None);
}

#[given(expr = "a config with provider {string}, api_key {string}, and api_base {string}")]
fn given_provider_config_with_api_base(
    world: &mut QuectoWorld,
    provider_name: String,
    api_key: String,
    api_base: String,
) {
    world.provider = providers::create_provider(&provider_name, api_key, Some(api_base));
}

#[when("I create a provider from config")]
fn when_create_provider(_world: &mut QuectoWorld) {
    let _ = &_world.provider;
}

#[then(expr = "the provider should be {string}")]
fn then_provider_is(world: &mut QuectoWorld, expected: String) {
    let provider = world.provider.as_ref().expect("no provider created");
    assert_eq!(provider.name(), expected);
}

#[then("no provider should be created")]
fn then_no_provider_created(world: &mut QuectoWorld) {
    assert!(world.provider.is_none(), "provider should not be created");
}

#[given(expr = "a provider error with status {int}")]
fn given_provider_error(world: &mut QuectoWorld, status: u16) {
    world.error_class = Some(ErrorClass::from_status(status));
}

#[then(expr = "the error should be classified as {string}")]
fn then_error_classified_as(world: &mut QuectoWorld, expected: String) {
    let class = world.error_class.as_ref().expect("no error class");
    assert_eq!(
        class.as_str(),
        expected,
        "expected error class '{}', got '{}'",
        expected,
        class.as_str()
    );
}

#[then("the error should be retryable")]
fn then_error_retryable(world: &mut QuectoWorld) {
    let class = world.error_class.as_ref().expect("no error class");
    assert!(class.is_retryable(), "expected error to be retryable");
}

#[then("the error should not be retryable")]
fn then_error_not_retryable(world: &mut QuectoWorld) {
    let class = world.error_class.as_ref().expect("no error class");
    assert!(!class.is_retryable(), "expected error to not be retryable");
}

// ===========================================================================
// Provider Fallback Steps
// ===========================================================================

/// A simple mock provider for BDD fallback tests that either succeeds or fails.
#[derive(Debug)]
struct BddTestProvider {
    provider_name: String,
    result: Mutex<Result<LlmResponse, String>>,
}

impl BddTestProvider {
    fn succeeding(name: &str, content: &str) -> Arc<Self> {
        Arc::new(Self {
            provider_name: name.to_string(),
            result: Mutex::new(Ok(LlmResponse {
                content: Some(content.to_string()),
                tool_calls: vec![],
                usage: None,
            })),
        })
    }

    fn failing(name: &str, error: &str) -> Arc<Self> {
        Arc::new(Self {
            provider_name: name.to_string(),
            result: Mutex::new(Err(error.to_string())),
        })
    }
}

impl LlmProvider for BddTestProvider {
    fn name(&self) -> &str {
        &self.provider_name
    }

    fn chat(
        &self,
        _request: quecto::domain::provider::ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        let result = self.result.lock().unwrap().clone();
        Box::pin(async move {
            match result {
                Ok(r) => Ok(r),
                Err(e) => Err(DomainError::Provider(e)),
            }
        })
    }
}

/// World fields for storing the primary/fallback providers before building FallbackProvider.
/// We store them as Vec since the FallbackProvider takes a vec.
static FALLBACK_PROVIDERS_KEY: &str = "_fallback_providers";

#[given(expr = "a primary provider that returns a server error {string}")]
fn given_primary_fails_server(world: &mut QuectoWorld, error: String) {
    let primary = BddTestProvider::failing("openai", &error) as Arc<dyn LlmProvider>;
    // Store in env_overrides as a sentinel; actual providers stored differently
    world
        .env_overrides
        .insert(FALLBACK_PROVIDERS_KEY.to_string(), "set".to_string());
    // We'll rebuild when creating the fallback provider
    world.provider = Some(primary);
}

#[given(expr = "a primary provider that returns a rate limit error {string}")]
fn given_primary_fails_rate_limit(world: &mut QuectoWorld, error: String) {
    let primary = BddTestProvider::failing("openai", &error) as Arc<dyn LlmProvider>;
    world
        .env_overrides
        .insert(FALLBACK_PROVIDERS_KEY.to_string(), "set".to_string());
    world.provider = Some(primary);
}

#[given(expr = "a fallback provider that returns {string}")]
fn given_fallback_that_returns(world: &mut QuectoWorld, content: String) {
    let primary = world
        .provider
        .take()
        .expect("primary provider must be set first");
    let fallback = BddTestProvider::succeeding("anthropic", &content) as Arc<dyn LlmProvider>;
    let fp = FallbackProvider::new(vec![primary, fallback]).with_cooldown_secs(60);
    world.fallback_provider = Some(Arc::new(fp));
}

#[when("I send a chat request through the fallback provider")]
fn when_send_through_fallback(world: &mut QuectoWorld) {
    let fp = world
        .fallback_provider
        .as_ref()
        .expect("fallback provider not set");
    let messages = vec![Message {
        role: Role::User,
        content: "test".to_string(),
        tool_calls: vec![],
        tool_call_id: None,
    }];
    let req = quecto::domain::provider::ChatRequest {
        messages: &messages,
        tools: &[],
        model: "test-model",
        max_tokens: 1024,
        temperature: 0.7,
    };
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(fp.chat(req))
        .expect("fallback chat should succeed");
    world.fallback_response = Some(result);
}

#[when("I send a second chat request through the fallback provider")]
fn when_send_second_through_fallback(world: &mut QuectoWorld) {
    // Same as above — the primary should be on cooldown, so it goes straight to fallback
    let fp = world
        .fallback_provider
        .as_ref()
        .expect("fallback provider not set");
    let messages = vec![Message {
        role: Role::User,
        content: "second test".to_string(),
        tool_calls: vec![],
        tool_call_id: None,
    }];
    let req = quecto::domain::provider::ChatRequest {
        messages: &messages,
        tools: &[],
        model: "test-model",
        max_tokens: 1024,
        temperature: 0.7,
    };
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(fp.chat(req))
        .expect("fallback chat should succeed on second call");
    world.fallback_response = Some(result);
}

#[then(expr = "the fallback response content should be {string}")]
fn then_fallback_response_content(world: &mut QuectoWorld, expected: String) {
    let response = world
        .fallback_response
        .as_ref()
        .expect("no fallback response");
    let content = response.content.as_ref().expect("response has no content");
    assert_eq!(
        content, &expected,
        "expected fallback response '{}', got '{}'",
        expected, content
    );
}

// ===========================================================================
// Provider Mock Server Steps (for real HTTP chat testing)
// ===========================================================================

#[given("an OpenAI provider with a mock server")]
fn given_openai_with_mock_server(world: &mut QuectoWorld) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(wiremock::MockServer::start());
    let uri = server.uri();

    // Provider created but will be replaced when mock response is configured
    world.provider = Some(Arc::new(
        quecto::infrastructure::providers::openai::OpenAiProvider::new(
            "sk-test-key".to_string(),
            Some(uri.clone()),
        ),
    ));
    world._wiremock_server_uri = Some(uri);
    std::mem::forget(server);
    std::mem::forget(rt);
}

#[given(expr = "the mock server returns a chat response with content {string}")]
fn given_mock_chat_response(world: &mut QuectoWorld, content: String) {
    // Create a fresh server with the mock already mounted
    let rt2 = tokio::runtime::Runtime::new().unwrap();
    let (uri2, _server2) = rt2.block_on(async {
        let server = wiremock::MockServer::start().await;
        let response_body = serde_json::json!({
            "id": "chatcmpl-test",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": content
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5,
                "total_tokens": 15
            }
        });
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .and(wiremock::matchers::header(
                "Authorization",
                "Bearer sk-test-key",
            ))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(response_body))
            .mount(&server)
            .await;
        let uri = server.uri();
        (uri, server)
    });

    // Recreate provider pointing at this mock
    world.provider = Some(Arc::new(
        quecto::infrastructure::providers::openai::OpenAiProvider::new(
            "sk-test-key".to_string(),
            Some(uri2.clone()),
        ),
    ));
    world._wiremock_server_uri = Some(uri2);
    std::mem::forget(_server2);
    std::mem::forget(rt2);
}

#[when(expr = "I send a chat request with message {string} and a tool {string}")]
fn when_send_chat_with_tool(world: &mut QuectoWorld, message: String, tool_name: String) {
    let provider = world.provider.as_ref().expect("provider not set");
    let messages = vec![Message {
        role: Role::User,
        content: message,
        tool_calls: vec![],
        tool_call_id: None,
    }];
    let tools = vec![quecto::domain::tool::ToolDefinition {
        name: tool_name,
        description: "Execute a command".to_string(),
        parameters_schema: r#"{"type":"object","properties":{"command":{"type":"string"}}}"#
            .to_string(),
    }];
    let req = quecto::domain::provider::ChatRequest {
        messages: &messages,
        tools: &tools,
        model: "gpt-4",
        max_tokens: 1024,
        temperature: 0.7,
    };
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(provider.chat(req));
    match result {
        Ok(response) => {
            world.fallback_response = Some(response);
        }
        Err(e) => {
            panic!("chat request failed: {}", e);
        }
    }
}

#[then(expr = "the chat response content should be {string}")]
fn then_chat_response_content(world: &mut QuectoWorld, expected: String) {
    let response = world.fallback_response.as_ref().expect("no chat response");
    let content = response.content.as_ref().expect("response has no content");
    assert_eq!(
        content, &expected,
        "expected chat response '{}', got '{}'",
        expected, content
    );
}

#[then("the chat request should have included an Authorization header")]
fn then_chat_had_auth_header(world: &mut QuectoWorld) {
    // The mock server requires an exact `Authorization: Bearer sk-test-key` header
    // (via wiremock::matchers::header on the mock setup). If the provider omits or
    // sends the wrong header, the mock returns no match and the request fails.
    // A successful response with content therefore proves the header was sent.
    let response = world
        .fallback_response
        .as_ref()
        .expect("no chat response — provider may not have sent the Authorization header");
    assert!(
        response.content.is_some(),
        "mock server requires Authorization header; no content means the header was missing or wrong"
    );
}

// ===========================================================================
// Streaming Provider Steps
// ===========================================================================

#[given("an Anthropic provider with a mock server")]
fn given_anthropic_with_mock_server(world: &mut QuectoWorld) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(wiremock::MockServer::start());
    let uri = server.uri();

    world.provider = Some(Arc::new(
        quecto::infrastructure::providers::anthropic::AnthropicProvider::new(
            "sk-ant-test-key".to_string(),
            Some(uri.clone()),
        ),
    ));
    world._wiremock_server_uri = Some(uri);
    std::mem::forget(server);
    std::mem::forget(rt);
}

#[given(expr = "the mock server returns an OpenAI streaming response with content {string}")]
fn given_openai_streaming_response(world: &mut QuectoWorld, content: String) {
    // Build SSE payload from the content string
    let words: Vec<&str> = content.split_whitespace().collect();
    let mut sse = String::new();
    for (i, word) in words.iter().enumerate() {
        let prefix = if i > 0 { " " } else { "" };
        sse.push_str(&format!(
            "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{}{}\"}}}}]}}\n\n",
            prefix, word
        ));
    }
    sse.push_str("data: [DONE]\n\n");

    // Start a new mock server with the SSE response
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (uri, _server) = rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_string(sse)
                    .insert_header("content-type", "text/event-stream"),
            )
            .mount(&server)
            .await;
        let uri = server.uri();
        (uri, server)
    });

    world.provider = Some(Arc::new(
        quecto::infrastructure::providers::openai::OpenAiProvider::new(
            "sk-test-key".to_string(),
            Some(uri.clone()),
        ),
    ));
    world._wiremock_server_uri = Some(uri);
    std::mem::forget(_server);
    std::mem::forget(rt);
}

#[given(expr = "the mock server returns an Anthropic streaming response with content {string}")]
fn given_anthropic_streaming_response(world: &mut QuectoWorld, content: String) {
    let words: Vec<&str> = content.split_whitespace().collect();
    let mut sse = String::new();
    for (i, word) in words.iter().enumerate() {
        let prefix = if i > 0 { " " } else { "" };
        sse.push_str(&format!(
            "event: content_block_delta\ndata: {{\"delta\":{{\"type\":\"text_delta\",\"text\":\"{}{}\"}}}}\n\n",
            prefix, word
        ));
    }
    sse.push_str("event: message_stop\ndata: {}\n\n");

    let rt = tokio::runtime::Runtime::new().unwrap();
    let (uri, _server) = rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/v1/messages"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_string(sse)
                    .insert_header("content-type", "text/event-stream"),
            )
            .mount(&server)
            .await;
        let uri = server.uri();
        (uri, server)
    });

    world.provider = Some(Arc::new(
        quecto::infrastructure::providers::anthropic::AnthropicProvider::new(
            "sk-ant-test-key".to_string(),
            Some(uri.clone()),
        ),
    ));
    world._wiremock_server_uri = Some(uri);
    std::mem::forget(_server);
    std::mem::forget(rt);
}

#[when(expr = "I send a streaming chat request with message {string}")]
fn when_send_streaming_chat(world: &mut QuectoWorld, message: String) {
    let provider = world.provider.as_ref().expect("provider not set");
    let messages = vec![Message {
        role: Role::User,
        content: message,
        tool_calls: vec![],
        tool_call_id: None,
    }];
    let req = quecto::domain::provider::ChatRequest {
        messages: &messages,
        tools: &[],
        model: "test-model",
        max_tokens: 1024,
        temperature: 0.7,
    };
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt
        .block_on(provider.chat_stream(req))
        .expect("streaming chat should succeed");
    world.streaming_response = Some(result);
}

#[then(expr = "the streaming response content should be {string}")]
fn then_streaming_response_content(world: &mut QuectoWorld, expected: String) {
    let response = world
        .streaming_response
        .as_ref()
        .expect("no streaming response");
    let content = response
        .content
        .as_ref()
        .expect("streaming response has no content");
    assert_eq!(
        content, &expected,
        "expected streaming content '{}', got '{}'",
        expected, content
    );
}

// ===========================================================================
