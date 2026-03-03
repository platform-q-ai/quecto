use super::*;

// Provider Steps
// ===========================================================================

#[given(expr = "a config with provider {string} and api_key {string}")]
fn given_provider_config(world: &mut QuectoWorld, provider_name: String, api_key: String) {
    world.provider = providers::create_provider(&provider_name, api_key, None).ok();
}

#[given(expr = "a config with provider {string}, api_key {string}, and api_base {string}")]
fn given_provider_config_with_api_base(
    world: &mut QuectoWorld,
    provider_name: String,
    api_key: String,
    api_base: String,
) {
    world.provider = providers::create_provider(&provider_name, api_key, Some(api_base)).ok();
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
                stop_reason: None,
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
    let messages = vec![Message::user("test")];
    let req = quecto::domain::provider::ChatRequest {
        messages: &messages,
        tools: &[],
        model: "test-model",
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
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
    let messages = vec![Message::user("second test")];
    let req = quecto::domain::provider::ChatRequest {
        messages: &messages,
        tools: &[],
        model: "test-model",
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
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
    let messages = vec![Message::user(message)];
    let tools = vec![quecto::domain::tool::ToolDefinition {
        name: tool_name.into(),
        description: "Execute a command".into(),
        parameters_schema: r#"{"type":"object","properties":{"command":{"type":"string"}}}"#.into(),
    }];
    let req = quecto::domain::provider::ChatRequest {
        messages: &messages,
        tools: &tools,
        model: "gpt-4",
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
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
    let messages = vec![Message::user(message)];
    let req = quecto::domain::provider::ChatRequest {
        messages: &messages,
        tools: &[],
        model: "test-model",
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
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
// Model Routing Steps
// ===========================================================================

/// A tracking provider that records the model it received and which provider name responded.
#[derive(Debug)]
struct RoutingTracker {
    name: String,
    response: Mutex<Result<LlmResponse, String>>,
}

impl RoutingTracker {
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

impl LlmProvider for RoutingTracker {
    fn name(&self) -> &str {
        &self.name
    }

    fn chat(
        &self,
        _request: quecto::domain::provider::ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        let result = self.response.lock().unwrap().clone();
        let name = self.name.clone();
        Box::pin(async move {
            match result {
                Ok(mut r) => {
                    // Embed handler name in content for step verification: "[provider_name] ..."
                    r.content = Some(format!("[{}] {}", name, r.content.unwrap_or_default()));
                    Ok(r)
                }
                Err(e) => Err(DomainError::Provider(e)),
            }
        })
    }
}

#[given("a fallback provider with OpenAI first and Anthropic second")]
fn given_fallback_openai_then_anthropic(world: &mut QuectoWorld) {
    let openai = RoutingTracker::succeeding("openai", "response") as Arc<dyn LlmProvider>;
    let anthropic = RoutingTracker::succeeding("anthropic", "response") as Arc<dyn LlmProvider>;
    let fp = FallbackProvider::new(vec![openai, anthropic]);
    world.fallback_provider = Some(Arc::new(fp));
}

#[given("a fallback provider with a failing OpenAI and a succeeding Anthropic")]
fn given_fallback_failing_openai_succeeding_anthropic(world: &mut QuectoWorld) {
    let openai =
        RoutingTracker::failing("openai", "HTTP 500 Internal Server Error") as Arc<dyn LlmProvider>;
    let anthropic =
        RoutingTracker::succeeding("anthropic", "Claude response") as Arc<dyn LlmProvider>;
    let fp = FallbackProvider::new(vec![openai, anthropic]);
    world.fallback_provider = Some(Arc::new(fp));
}

#[when(expr = "I send a chat request with model {string}")]
fn when_send_chat_with_model(world: &mut QuectoWorld, model: String) {
    let fp = world
        .fallback_provider
        .as_ref()
        .expect("fallback provider not set");
    let messages = vec![Message::user("test message")];
    let req = quecto::domain::provider::ChatRequest {
        messages: &messages,
        tools: &[],
        model: &model,
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
    };
    let rt = tokio::runtime::Runtime::new().unwrap();
    match rt.block_on(fp.chat(req)) {
        Ok(response) => {
            // Extract handler name from embedded content "[provider_name] ..."
            let content = response.content.clone().unwrap_or_default();
            let handled_by = if content.starts_with('[') {
                if let Some(end) = content.find(']') {
                    content[1..end].to_string()
                } else {
                    String::new()
                }
            } else {
                String::new()
            };
            world.routing_handled_by = Some(handled_by);
            world.routing_succeeded = Some(true);
            world.routing_response = Some(response);
        }
        Err(_) => {
            world.routing_handled_by = None;
            world.routing_succeeded = Some(false);
        }
    }
}

#[then(expr = "the request should be handled by the {string} provider")]
fn then_handled_by_provider(world: &mut QuectoWorld, expected: String) {
    let handled_by = world
        .routing_handled_by
        .as_deref()
        .expect("no routing result — request may have failed");
    assert_eq!(
        handled_by, expected,
        "expected model to be routed to '{}' but was handled by '{}'",
        expected, handled_by
    );
}

#[then("the request should succeed with the Anthropic response")]
fn then_request_succeeds_with_anthropic(world: &mut QuectoWorld) {
    assert!(
        world.routing_succeeded == Some(true),
        "expected routing to succeed but it failed"
    );
    let handled_by = world
        .routing_handled_by
        .as_deref()
        .expect("no routing handler recorded");
    assert_eq!(
        handled_by, "anthropic",
        "expected Anthropic to handle the request, got '{}'",
        handled_by
    );
}

// ===========================================================================
// #178: is_error flag on tool result messages
// ===========================================================================

#[given("an Anthropic request with a tool result marked as error")]
fn given_error_tool_result(world: &mut QuectoWorld) {
    let mut m = Message::tool("tc_1", "Error: file not found");
    m.is_error = true;
    world.context_messages = Some(vec![m]);
}

#[given("an Anthropic request with a successful tool result")]
fn given_success_tool_result(world: &mut QuectoWorld) {
    let mut m = Message::tool("tc_1", "file contents");
    m.is_error = false;
    world.context_messages = Some(vec![m]);
}

#[when("I build the Anthropic tool result message")]
fn when_build_tool_result(world: &mut QuectoWorld) {
    let msgs = world.context_messages.as_ref().expect("no messages set");
    let json =
        quecto::infrastructure::providers::anthropic::AnthropicProvider::build_tool_result_message_public(
            &msgs[0],
        );
    world
        .env_overrides
        .insert("_anthropic_json".into(), json.to_string());
}

#[then(expr = "the tool result JSON should contain {string} set to true")]
fn then_tool_result_json_has_field_true(world: &mut QuectoWorld, field: String) {
    let json_str = world.env_overrides.get("_anthropic_json").expect("no json");
    let json: serde_json::Value = serde_json::from_str(json_str).expect("invalid json");
    let content = json["content"].as_array().expect("content should be array");
    assert_eq!(content[0][&field], serde_json::Value::Bool(true));
}

#[then(expr = "the tool result JSON should contain {string} set to false")]
fn then_tool_result_json_has_field_false(world: &mut QuectoWorld, field: String) {
    let json_str = world.env_overrides.get("_anthropic_json").expect("no json");
    let json: serde_json::Value = serde_json::from_str(json_str).expect("invalid json");
    let content = json["content"].as_array().expect("content should be array");
    assert_eq!(content[0][&field], serde_json::Value::Bool(false));
}

// ===========================================================================
// #179: Beta headers for API key auth
// ===========================================================================

#[given("an Anthropic provider with API key auth and a mock server")]
fn given_anthropic_api_key_mock(world: &mut QuectoWorld) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(wiremock::MockServer::start());
    let uri = server.uri();

    // Set up mock that requires the beta header
    let response_body = serde_json::json!({
        "id": "msg_beta",
        "type": "message",
        "role": "assistant",
        "content": [{"type": "text", "text": "ok"}],
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 5, "output_tokens": 2}
    });
    rt.block_on(async {
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/v1/messages"))
            .and(wiremock::matchers::header(
                "anthropic-beta",
                "fine-grained-tool-streaming-2025-05-14",
            ))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(response_body))
            .mount(&server)
            .await;
    });

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

#[when("I send an Anthropic chat request")]
fn when_send_anthropic_chat(world: &mut QuectoWorld) {
    let provider = world.provider.as_ref().expect("provider not set");
    let messages = vec![Message::user("Hi")];
    let req = quecto::domain::provider::ChatRequest {
        messages: &messages,
        tools: &[],
        model: "claude-sonnet-4-20250514",
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
    };
    let rt = tokio::runtime::Runtime::new().unwrap();
    match rt.block_on(provider.chat(req)) {
        Ok(response) => {
            world.streaming_response = Some(response);
        }
        Err(e) => panic!("Anthropic chat failed: {}", e),
    }
}

#[then(expr = "the request should include the {string} header with {string}")]
fn then_request_has_header(world: &mut QuectoWorld, _header: String, _value: String) {
    // If the mock matched (which requires the header), and we got a response, then the header was sent.
    assert!(
        world.streaming_response.is_some(),
        "no response — the header may not have been sent"
    );
}

// ===========================================================================
// #177: Stop reason extraction
// ===========================================================================

#[given(expr = "an Anthropic mock server that returns stop_reason {string}")]
fn given_anthropic_mock_stop_reason(world: &mut QuectoWorld, stop_reason: String) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (uri, _server) = rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let response_body = serde_json::json!({
            "id": "msg_stop",
            "type": "message",
            "role": "assistant",
            "content": [{"type": "text", "text": "Hello"}],
            "stop_reason": stop_reason,
            "usage": {"input_tokens": 10, "output_tokens": 5}
        });
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/v1/messages"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(response_body))
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

#[given(expr = "an Anthropic mock server that streams with stop_reason {string}")]
fn given_anthropic_mock_sse_stop_reason(world: &mut QuectoWorld, stop_reason: String) {
    let sse = format!(
        "event: content_block_delta\n\
         data: {{\"delta\":{{\"type\":\"text_delta\",\"text\":\"Hello\"}}}}\n\n\
         event: message_delta\n\
         data: {{\"delta\":{{\"stop_reason\":\"{}\"}},\"usage\":{{\"output_tokens\":10}}}}\n\n\
         event: message_stop\n\
         data: {{}}\n\n",
        stop_reason
    );

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

#[when("I send an Anthropic streaming chat request")]
fn when_send_anthropic_streaming(world: &mut QuectoWorld) {
    let provider = world.provider.as_ref().expect("provider not set");
    let messages = vec![Message::user("Hi")];
    let req = quecto::domain::provider::ChatRequest {
        messages: &messages,
        tools: &[],
        model: "claude-sonnet-4-20250514",
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
    };
    let rt = tokio::runtime::Runtime::new().unwrap();
    match rt.block_on(provider.chat_stream(req)) {
        Ok(response) => {
            world.streaming_response = Some(response);
        }
        Err(e) => panic!("Anthropic streaming chat failed: {}", e),
    }
}

#[then(expr = "the response stop_reason should be {string}")]
fn then_stop_reason_is(world: &mut QuectoWorld, expected: String) {
    let response = world.streaming_response.as_ref().expect("no response");
    let stop_reason = response
        .stop_reason
        .as_ref()
        .expect("response has no stop_reason");
    let actual = format!("{:?}", stop_reason);
    assert_eq!(
        actual, expected,
        "expected stop_reason {:?}, got {:?}",
        expected, actual
    );
}

// ===========================================================================
// #180: Usage from SSE stream
// ===========================================================================

#[given(expr = "an Anthropic mock server that streams usage with input {int} and output {int}")]
fn given_anthropic_mock_sse_usage(world: &mut QuectoWorld, input: u32, output: u32) {
    let sse = format!(
        "event: message_start\n\
         data: {{\"message\":{{\"usage\":{{\"input_tokens\":{},\"output_tokens\":0}}}}}}\n\n\
         event: content_block_delta\n\
         data: {{\"delta\":{{\"type\":\"text_delta\",\"text\":\"Hi\"}}}}\n\n\
         event: message_delta\n\
         data: {{\"delta\":{{\"stop_reason\":\"end_turn\"}},\"usage\":{{\"output_tokens\":{}}}}}\n\n\
         event: message_stop\n\
         data: {{}}\n\n",
        input, output
    );

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

#[given(expr = "an Anthropic mock server that streams cache usage with read {int} and write {int}")]
fn given_anthropic_mock_sse_cache_usage(
    world: &mut QuectoWorld,
    cache_read: u32,
    cache_write: u32,
) {
    let sse = format!(
        "event: message_start\n\
         data: {{\"message\":{{\"usage\":{{\"input_tokens\":100,\"output_tokens\":0,\"cache_read_input_tokens\":{},\"cache_creation_input_tokens\":{}}}}}}}\n\n\
         event: content_block_delta\n\
         data: {{\"delta\":{{\"type\":\"text_delta\",\"text\":\"Hi\"}}}}\n\n\
         event: message_delta\n\
         data: {{\"delta\":{{\"stop_reason\":\"end_turn\"}},\"usage\":{{\"output_tokens\":50}}}}\n\n\
         event: message_stop\n\
         data: {{}}\n\n",
        cache_read, cache_write
    );

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

#[then(expr = "the response usage should have prompt_tokens {int} and completion_tokens {int}")]
fn then_usage_has_tokens(world: &mut QuectoWorld, expected_prompt: u32, expected_completion: u32) {
    let response = world.streaming_response.as_ref().expect("no response");
    let usage = response.usage.as_ref().expect("response has no usage");
    assert_eq!(usage.prompt_tokens, expected_prompt);
    assert_eq!(usage.completion_tokens, expected_completion);
}

#[then(
    expr = "the response usage should have cache_read_tokens {int} and cache_write_tokens {int}"
)]
fn then_usage_has_cache_tokens(world: &mut QuectoWorld, expected_read: u32, expected_write: u32) {
    let response = world.streaming_response.as_ref().expect("no response");
    let usage = response.usage.as_ref().expect("response has no usage");
    assert_eq!(usage.cache_read_tokens, Some(expected_read));
    assert_eq!(usage.cache_write_tokens, Some(expected_write));
}

// ===========================================================================
// #176: Prompt caching (cache_control markers)
// ===========================================================================

#[given(expr = "an Anthropic request with a system prompt {string}")]
fn given_anthropic_request_with_system_prompt(world: &mut QuectoWorld, prompt: String) {
    world.context_messages = Some(vec![Message::system(prompt), Message::user("Hi")]);
}

#[given("an Anthropic request with multiple user messages")]
fn given_anthropic_request_multiple_user_msgs(world: &mut QuectoWorld) {
    world.context_messages = Some(vec![
        Message::system("You are helpful."),
        Message::user("First"),
        Message::user("Second"),
    ]);
}

#[when("I build the Anthropic request body")]
fn when_build_anthropic_request_body(world: &mut QuectoWorld) {
    let msgs = world.context_messages.as_ref().expect("no messages set");
    let req = quecto::domain::provider::ChatRequest {
        messages: msgs,
        tools: &[],
        model: "claude-sonnet-4-20250514",
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
    };
    let (_sys, body) =
        quecto::infrastructure::providers::anthropic::AnthropicProvider::build_request_body_public(
            &req,
        );
    world
        .env_overrides
        .insert("_anthropic_body".into(), body.to_string());
}

#[then("the system prompt should be a content block array with cache_control")]
fn then_system_prompt_has_cache_control(world: &mut QuectoWorld) {
    let body_str = world.env_overrides.get("_anthropic_body").expect("no body");
    let body: serde_json::Value = serde_json::from_str(body_str).expect("invalid json");
    let system = &body["system"];
    assert!(system.is_array(), "system should be content block array");
    let blocks = system.as_array().unwrap();
    assert!(!blocks.is_empty());
    assert_eq!(blocks[0]["type"], "text");
    assert!(blocks[0]["cache_control"].is_object());
    assert_eq!(blocks[0]["cache_control"]["type"], "ephemeral");
}

#[then("the last user message content block should have cache_control")]
fn then_last_user_msg_has_cache_control(world: &mut QuectoWorld) {
    let body_str = world.env_overrides.get("_anthropic_body").expect("no body");
    let body: serde_json::Value = serde_json::from_str(body_str).expect("invalid json");
    let messages = body["messages"]
        .as_array()
        .expect("messages should be array");
    let last_msg = messages.last().expect("should have messages");
    let content = &last_msg["content"];
    assert!(content.is_array(), "last user msg content should be array");
    let blocks = content.as_array().unwrap();
    let last_block = blocks.last().unwrap();
    assert!(last_block["cache_control"].is_object());
    assert_eq!(last_block["cache_control"]["type"], "ephemeral");
}

// ===========================================================================
// #187: Batch consecutive tool results
// ===========================================================================

#[given(expr = "an Anthropic request with {int} consecutive tool result messages")]
fn given_consecutive_tool_results(world: &mut QuectoWorld, count: usize) {
    let mut msgs = vec![
        Message::user("do stuff"),
        Message::assistant(
            "",
            (0..count)
                .map(|i| ToolCall {
                    id: format!("tc_{}", i),
                    name: "bash".into(),
                    arguments: "{}".into(),
                })
                .collect(),
        ),
    ];
    for i in 0..count {
        msgs.push(Message::tool(format!("tc_{}", i), format!("output {}", i)));
    }
    world.context_messages = Some(msgs);
}

#[when("I build the Anthropic messages")]
fn when_build_anthropic_messages(world: &mut QuectoWorld) {
    let msgs = world.context_messages.as_ref().expect("no messages set");
    let (_sys, api_msgs) =
        quecto::infrastructure::providers::anthropic::AnthropicProvider::build_messages_public(
            msgs,
        );
    world.env_overrides.insert(
        "_anthropic_msgs".into(),
        serde_json::to_string(&api_msgs).unwrap(),
    );
}

#[then(
    expr = "the tool results should be batched into a single user message with {int} tool_result blocks"
)]
fn then_tool_results_batched(world: &mut QuectoWorld, expected_count: usize) {
    let msgs_str = world.env_overrides.get("_anthropic_msgs").expect("no msgs");
    let msgs: Vec<serde_json::Value> = serde_json::from_str(msgs_str).expect("invalid json");
    // Find the user message with tool_result content
    let tool_msg = msgs
        .iter()
        .find(|m| {
            m["role"] == "user"
                && m["content"]
                    .as_array()
                    .map(|arr| arr.iter().any(|b| b["type"] == "tool_result"))
                    .unwrap_or(false)
        })
        .expect("no user message with tool_result blocks");
    let content = tool_msg["content"].as_array().unwrap();
    assert_eq!(content.len(), expected_count);
}

#[then(expr = "the tool result should be in a single user message with {int} tool_result block")]
fn then_tool_result_in_single_message(world: &mut QuectoWorld, expected_count: usize) {
    let msgs_str = world.env_overrides.get("_anthropic_msgs").expect("no msgs");
    let msgs: Vec<serde_json::Value> = serde_json::from_str(msgs_str).expect("invalid json");
    let tool_msg = msgs
        .iter()
        .find(|m| {
            m["role"] == "user"
                && m["content"]
                    .as_array()
                    .map(|arr| arr.iter().any(|b| b["type"] == "tool_result"))
                    .unwrap_or(false)
        })
        .expect("no user message with tool_result blocks");
    let content = tool_msg["content"].as_array().unwrap();
    assert_eq!(content.len(), expected_count);
}

// ===========================================================================
// #183: tool_choice parameter
// ===========================================================================

#[given(expr = "an Anthropic request with tool_choice {string}")]
fn given_anthropic_tool_choice(world: &mut QuectoWorld, choice: String) {
    let tc = match choice.as_str() {
        "auto" => quecto::domain::provider::ToolChoice::Auto,
        "any" => quecto::domain::provider::ToolChoice::Any,
        _ => panic!("unknown tool_choice: {}", choice),
    };
    world.env_overrides.insert("_tool_choice".into(), choice);
    world.context_messages = Some(vec![Message::user("Hi")]);
    // Store the tool_choice for later use
    // (Using env_overrides as simple key-value store for BDD state)
    world
        .env_overrides
        .insert("_tool_choice_type".into(), format!("{:?}", tc));
}

#[given(expr = "an Anthropic request with tool_choice for tool {string}")]
fn given_anthropic_tool_choice_specific(world: &mut QuectoWorld, tool_name: String) {
    world
        .env_overrides
        .insert("_tool_choice".into(), format!("specific:{}", tool_name));
    world.context_messages = Some(vec![Message::user("Hi")]);
}

#[when("I build the Anthropic request body with tool_choice")]
fn when_build_with_tool_choice(world: &mut QuectoWorld) {
    let msgs = world.context_messages.as_ref().expect("no messages set");
    let choice_str = world
        .env_overrides
        .get("_tool_choice")
        .expect("no tool_choice");
    let tool_choice = if choice_str.starts_with("specific:") {
        let name = choice_str.strip_prefix("specific:").unwrap();
        Some(quecto::domain::provider::ToolChoice::Specific(name.into()))
    } else {
        match choice_str.as_str() {
            "auto" => Some(quecto::domain::provider::ToolChoice::Auto),
            "any" => Some(quecto::domain::provider::ToolChoice::Any),
            _ => None,
        }
    };
    let tools = vec![quecto::domain::tool::ToolDefinition {
        name: "bash".into(),
        description: "Execute".into(),
        parameters_schema: "{}".into(),
    }];
    let req = quecto::domain::provider::ChatRequest {
        messages: msgs,
        tools: &tools,
        model: "claude-sonnet-4-20250514",
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice,
        metadata: None,
    };
    let (_sys, body) =
        quecto::infrastructure::providers::anthropic::AnthropicProvider::build_request_body_public(
            &req,
        );
    world
        .env_overrides
        .insert("_anthropic_body".into(), body.to_string());
}

#[then(expr = "the request body should contain tool_choice type {string}")]
fn then_tool_choice_type(world: &mut QuectoWorld, expected_type: String) {
    let body_str = world.env_overrides.get("_anthropic_body").expect("no body");
    let body: serde_json::Value = serde_json::from_str(body_str).expect("invalid json");
    assert_eq!(body["tool_choice"]["type"], expected_type);
}

#[then(expr = "the request body should contain tool_choice type {string} with name {string}")]
fn then_tool_choice_type_with_name(
    world: &mut QuectoWorld,
    expected_type: String,
    expected_name: String,
) {
    let body_str = world.env_overrides.get("_anthropic_body").expect("no body");
    let body: serde_json::Value = serde_json::from_str(body_str).expect("invalid json");
    assert_eq!(body["tool_choice"]["type"], expected_type);
    assert_eq!(body["tool_choice"]["name"], expected_name);
}

// ===========================================================================
// #186: metadata.user_id support
// ===========================================================================

#[given(expr = "an Anthropic request with user_id {string}")]
fn given_anthropic_metadata_user_id(world: &mut QuectoWorld, user_id: String) {
    world
        .env_overrides
        .insert("_metadata_user_id".into(), user_id);
    world.context_messages = Some(vec![Message::user("Hi")]);
}

#[given("an Anthropic request without metadata")]
fn given_anthropic_no_metadata(world: &mut QuectoWorld) {
    world.env_overrides.remove("_metadata_user_id");
    world.context_messages = Some(vec![Message::user("Hi")]);
}

#[when("I build the Anthropic request body with metadata")]
fn when_build_with_metadata(world: &mut QuectoWorld) {
    let msgs = world.context_messages.as_ref().expect("no messages set");
    let metadata = world.env_overrides.get("_metadata_user_id").map(|uid| {
        quecto::domain::provider::RequestMetadata {
            user_id: Some(uid.clone()),
        }
    });
    let req = quecto::domain::provider::ChatRequest {
        messages: msgs,
        tools: &[],
        model: "claude-sonnet-4-20250514",
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata,
    };
    let (_sys, body) =
        quecto::infrastructure::providers::anthropic::AnthropicProvider::build_request_body_public(
            &req,
        );
    world
        .env_overrides
        .insert("_anthropic_body".into(), body.to_string());
}

#[then(expr = "the request body should contain metadata with user_id {string}")]
fn then_metadata_user_id(world: &mut QuectoWorld, expected_user_id: String) {
    let body_str = world.env_overrides.get("_anthropic_body").expect("no body");
    let body: serde_json::Value = serde_json::from_str(body_str).expect("invalid json");
    assert_eq!(body["metadata"]["user_id"], expected_user_id);
}

#[then("the request body should not contain a metadata field")]
fn then_no_metadata(world: &mut QuectoWorld) {
    let body_str = world.env_overrides.get("_anthropic_body").expect("no body");
    let body: serde_json::Value = serde_json::from_str(body_str).expect("invalid json");
    assert!(
        body.get("metadata").is_none() || body["metadata"].is_null(),
        "metadata should not be present"
    );
}
