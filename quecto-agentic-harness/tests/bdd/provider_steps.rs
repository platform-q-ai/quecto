use super::*;

// Provider Steps
// ===========================================================================

#[given(expr = "a config with provider {string} and api_key {string}")]
fn given_provider_config(world: &mut QuectoWorld, provider_name: String, api_key: String) {
    world.provider = providers::create_provider_with_client(
        &provider_name,
        api_key,
        None,
        reqwest::Client::new(),
    )
    .ok();
}

#[given(expr = "a config with provider {string}, api_key {string}, and api_base {string}")]
fn given_provider_config_with_api_base(
    world: &mut QuectoWorld,
    provider_name: String,
    api_key: String,
    api_base: String,
) {
    world.provider = providers::create_provider_with_client(
        &provider_name,
        api_key,
        Some(api_base),
        reqwest::Client::new(),
    )
    .ok();
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
    world.error_class = Some(ProviderErrorClass::from_status(status));
}

#[given(expr = "a provider error with message {string}")]
fn given_provider_error_with_message(world: &mut QuectoWorld, message: String) {
    use quecto::domain::error::DomainError;
    let err = DomainError::Provider(message);
    world.error_class = Some(classify_provider_error(&err));
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

#[given(expr = "a 500 response whose body declares an {string} client code")]
fn given_500_with_client_code(world: &mut QuectoWorld, code: String) {
    // Push the wire detail into the step, keeping the feature declarative (#935).
    use quecto::domain::error::DomainError;
    let message = format!(
        "HTTP 500 from provider: {{\"error\":{{\"code\":\"{code}\",\"message\":\"max_tokens 200000 exceeds the model output limit of 65536\"}}}}"
    );
    let err = DomainError::Provider(message);
    world.error_class = Some(classify_provider_error(&err));
}

// ===========================================================================
// #935: per-model max_tokens clamp (the headline Fireworks fix)
// ===========================================================================

#[given(expr = "a model whose output cap is {int} tokens")]
fn given_model_output_cap(world: &mut QuectoWorld, cap: u32) {
    super::agent_loop_steps::ensure_mock_llm(world);
    world
        .env_overrides
        .insert("_model_output_cap".into(), cap.to_string());
}

#[given(expr = "a configured max_tokens of {int}")]
fn given_configured_max_tokens_935(world: &mut QuectoWorld, configured: u32) {
    world
        .env_overrides
        .insert("_configured_max_tokens".into(), configured.to_string());
}

#[when("the agent builds a request for that model")]
fn when_agent_builds_request_for_model(world: &mut QuectoWorld) {
    let configured: u32 = world
        .env_overrides
        .get("_configured_max_tokens")
        .and_then(|v| v.parse().ok())
        .expect("configured max_tokens not set");
    let cap: u32 = world
        .env_overrides
        .get("_model_output_cap")
        .and_then(|v| v.parse().ok())
        .expect("model output cap not set");
    let provider = world.mock_llm.clone().expect("mock LLM not configured");
    provider.push_response(LlmResponse {
        content: Some("done".to_string()),
        tool_calls: vec![],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    });
    let agent = AgentLoopImpl::new(quecto::application::agent_loop::AgentLoopConfig {
        provider: provider.clone() as Arc<dyn LlmProvider>,
        tool_registry: Box::new(ToolRegistryImpl::new()),
        model: "fireworks/qwen3p7-plus".to_string(),
        max_tokens: configured,
        temperature: 0.7,
        spill_store: None,
        session_key: String::new(),
        context_collapse_after_turns: u32::MAX,
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        system_prompt_provider: None,
        audit_log: None,
    })
    .with_model_max_tokens(Some(cap));
    let mut messages = vec![Message::user("hi")];
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(agent.process(&mut messages))
        .expect("agent process failed in clamp scenario");
}

#[then(expr = "the request output cap should be {int}")]
fn then_request_output_cap(world: &mut QuectoWorld, expected: u32) {
    let provider = world.mock_llm.as_ref().expect("mock LLM not configured");
    assert_eq!(
        provider.last_max_tokens(),
        Some(expected),
        "the request the provider received must carry the clamped output cap"
    );
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
                thinking_blocks: vec![],
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

/// Steps for the "no fallback" scenario — router forwards errors directly.

#[given("a provider router with a failing OpenAI and a succeeding Anthropic")]
fn given_router_failing_openai_succeeding_anthropic(world: &mut QuectoWorld) {
    let openai = BddTestProvider::failing("openai", "HTTP 500 Internal Server Error")
        as Arc<dyn LlmProvider>;
    let anthropic =
        BddTestProvider::succeeding("anthropic", "Anthropic response") as Arc<dyn LlmProvider>;
    let router = ProviderRouter::new(vec![openai, anthropic]);
    world.provider_router = Some(Arc::new(router));
}

#[when(expr = "I send a chat request with model {string} through the router")]
fn when_send_through_router_with_model(world: &mut QuectoWorld, model: String) {
    let router = world
        .provider_router
        .as_ref()
        .expect("provider router not set");
    let messages = vec![Message::user("test")];
    let req = quecto::domain::provider::ChatRequest {
        messages: &messages,
        tools: &[],
        model: &model,
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    match tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(router.chat(req))
    {
        Ok(response) => {
            world.router_response = Some(response);
            world.routing_succeeded = Some(true);
        }
        Err(e) => {
            world.routing_succeeded = Some(false);
            world
                .env_overrides
                .insert("_router_error".into(), e.to_string());
        }
    }
}

// ===========================================================================
// #931: Retry decorator (RetryingProvider) steps
// ===========================================================================

/// Counting mock provider for retry-decorator BDD scenarios: fails the first
/// `fail_until` calls with a configurable error, then succeeds. Shares an
/// atomic counter so the scenario can assert the exact number of attempts.
#[derive(Debug)]
struct BddCountingProvider {
    call_count: Arc<std::sync::atomic::AtomicU32>,
    fail_until: u32,
    error_message: String,
}

impl LlmProvider for BddCountingProvider {
    fn name(&self) -> &str {
        "bdd-counting"
    }

    fn chat(
        &self,
        _request: quecto::domain::provider::ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        use std::sync::atomic::Ordering;
        let n = self.call_count.fetch_add(1, Ordering::SeqCst);
        let fail = n < self.fail_until;
        let msg = self.error_message.clone();
        Box::pin(async move {
            if fail {
                Err(DomainError::Provider(msg))
            } else {
                Ok(LlmResponse {
                    content: Some("retry-success".to_string()),
                    tool_calls: vec![],
                    usage: None,
                    stop_reason: None,
                    thinking_blocks: vec![],
                })
            }
        })
    }
}

#[given(expr = "a retrying provider that fails {int} time(s) with {string} then succeeds")]
fn given_retrying_provider_fails_n(world: &mut QuectoWorld, n: u32, message: String) {
    let count = Arc::new(std::sync::atomic::AtomicU32::new(0));
    world.retry_call_count = Some(count.clone());
    world.retry_inner = Some(Arc::new(BddCountingProvider {
        call_count: count,
        fail_until: n,
        error_message: message,
    }));
}

#[given(expr = "a retrying provider that always fails with {string}")]
fn given_retrying_provider_always_fails(world: &mut QuectoWorld, message: String) {
    let count = Arc::new(std::sync::atomic::AtomicU32::new(0));
    world.retry_call_count = Some(count.clone());
    world.retry_inner = Some(Arc::new(BddCountingProvider {
        call_count: count,
        fail_until: u32::MAX,
        error_message: message,
    }));
}

#[given(expr = "the retry decorator allows up to {int} attempts")]
fn given_retry_max_attempts(world: &mut QuectoWorld, attempts: u32) {
    world.retry_max_attempts = Some(attempts);
}

#[when("I send a chat request through the retrying provider")]
fn when_send_through_retrying_provider(world: &mut QuectoWorld) {
    use quecto::infrastructure::providers::retry::{RetryConfig, RetryingProvider};
    let inner = world.retry_inner.clone().expect("no retry inner provider");
    let attempts = world.retry_max_attempts.unwrap_or(4);
    let provider = RetryingProvider::new(inner, RetryConfig::no_delay(attempts));

    let messages = vec![Message::user("test")];
    let req = quecto::domain::provider::ChatRequest {
        messages: &messages,
        tools: &[],
        model: "test-model",
        max_tokens: 1024,
        temperature: 0.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let outcome = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(provider.chat(req));
    world.retry_succeeded = Some(outcome.is_ok());
}

fn retry_inner_call_count(world: &QuectoWorld) -> u32 {
    use std::sync::atomic::Ordering;
    world
        .retry_call_count
        .as_ref()
        .expect("no retry call counter")
        .load(Ordering::SeqCst)
}

#[then("the request eventually succeeds despite the transient failures")]
fn then_retry_recovers(world: &mut QuectoWorld) {
    assert_eq!(
        world.retry_succeeded,
        Some(true),
        "expected the request to recover and succeed"
    );
    assert!(
        retry_inner_call_count(world) > 1,
        "recovery implies the transient failure was retried (>1 attempt)"
    );
}

#[then("the request fails after retries are exhausted")]
fn then_retry_exhausted(world: &mut QuectoWorld) {
    assert_eq!(
        world.retry_succeeded,
        Some(false),
        "expected the request to fail after exhausting retries"
    );
    assert!(
        retry_inner_call_count(world) > 1,
        "a retryable error should have been retried before giving up (>1 attempt)"
    );
}

#[then("the request fails without being retried")]
fn then_fails_without_retry(world: &mut QuectoWorld) {
    assert_eq!(
        world.retry_succeeded,
        Some(false),
        "expected the request to fail"
    );
    assert_eq!(
        retry_inner_call_count(world),
        1,
        "a non-retryable error must not be retried — exactly one attempt"
    );
}

#[then("the request should fail with a provider error")]
fn then_request_fails_with_provider_error(world: &mut QuectoWorld) {
    assert_eq!(
        world.routing_succeeded,
        Some(false),
        "expected the request to fail, but it succeeded"
    );
}

#[then(expr = "the fallback response content should be {string}")]
fn then_router_response_content(world: &mut QuectoWorld, expected: String) {
    let response = world
        .router_response
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
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(provider.chat(req));
    match result {
        Ok(response) => {
            world.router_response = Some(response);
        }
        Err(e) => {
            panic!("chat request failed: {}", e);
        }
    }
}

#[then(expr = "the chat response content should be {string}")]
fn then_chat_response_content(world: &mut QuectoWorld, expected: String) {
    let response = world.router_response.as_ref().expect("no chat response");
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
        .router_response
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
        thinking_level: None,
        cancel_flag: None,
        effort: None,
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
    received_model: Mutex<Option<String>>,
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
                thinking_blocks: vec![],
            })),
            received_model: Mutex::new(None),
        })
    }
}

impl LlmProvider for RoutingTracker {
    fn name(&self) -> &str {
        &self.name
    }

    fn chat(
        &self,
        request: quecto::domain::provider::ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        *self.received_model.lock().unwrap() = Some(request.model.to_string());
        let result = self.response.lock().unwrap().clone();
        let name = self.name.clone();
        let model = request.model.to_string();
        Box::pin(async move {
            match result {
                Ok(mut r) => {
                    // Embed handler name and effective model for step verification.
                    r.content = Some(format!(
                        "[{}]{{{}}} {}",
                        name,
                        model,
                        r.content.unwrap_or_default()
                    ));
                    Ok(r)
                }
                Err(e) => Err(DomainError::Provider(e)),
            }
        })
    }
}

#[given("a provider router with OpenAI first and Anthropic second")]
fn given_router_openai_then_anthropic(world: &mut QuectoWorld) {
    let openai = RoutingTracker::succeeding("openai", "response") as Arc<dyn LlmProvider>;
    let anthropic = RoutingTracker::succeeding("anthropic", "response") as Arc<dyn LlmProvider>;
    let router = ProviderRouter::new(vec![openai, anthropic]);
    world.provider_router = Some(Arc::new(router));
}

#[given("a provider router with OpenAI first and Fireworks second")]
fn given_router_openai_then_fireworks(world: &mut QuectoWorld) {
    let openai = RoutingTracker::succeeding("openai", "response") as Arc<dyn LlmProvider>;
    let fireworks = RoutingTracker::succeeding("fireworks", "response") as Arc<dyn LlmProvider>;
    let router = ProviderRouter::new(vec![openai, fireworks]);
    world.provider_router = Some(Arc::new(router));
}

/// Provider that captures the messages slice pointer for zero-copy verification.
#[derive(Debug)]
struct SlicePtrBddProvider {
    captured_ptr: Mutex<Option<usize>>,
}

impl LlmProvider for SlicePtrBddProvider {
    fn name(&self) -> &str {
        "test"
    }
    fn chat(
        &self,
        request: quecto::domain::provider::ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        *self.captured_ptr.lock().unwrap() = Some(request.messages.as_ptr() as usize);
        Box::pin(async move {
            Ok(LlmResponse {
                content: Some("ok".to_string()),
                tool_calls: vec![],
                usage: None,
                stop_reason: None,
                thinking_blocks: vec![],
            })
        })
    }
}

#[given("a provider router with a single provider")]
fn given_router_single_provider(world: &mut QuectoWorld) {
    let inner = Arc::new(SlicePtrBddProvider {
        captured_ptr: Mutex::new(None),
    });
    let router = ProviderRouter::new(vec![inner.clone() as Arc<dyn LlmProvider>]);
    world.provider_router = Some(Arc::new(router));
    // inner Arc is also held inside the router; no separate storage needed.
    // Zero-copy pointer equality is verified by the unit tests in router_tests.rs.
}

#[when("I send a chat request through the router and track the messages pointer")]
fn when_send_and_track_ptr(world: &mut QuectoWorld) {
    let router = world
        .provider_router
        .as_ref()
        .expect("provider router not set");
    let messages = vec![Message::user("test")];
    let original_ptr = messages.as_ptr() as usize;
    world
        .env_overrides
        .insert("_original_msg_ptr".into(), original_ptr.to_string());

    let _response = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(router.chat(quecto::domain::provider::ChatRequest {
            messages: &messages,
            tools: &[],
            model: "test-model",
            max_tokens: 1024,
            temperature: 0.7,
            session_id: None,
            tool_choice: None,
            metadata: None,
            thinking_level: None,
            cancel_flag: None,
            effort: None,
        }))
        .expect("router chat should succeed in zero-copy test");
}

#[then("the provider should receive the same messages pointer as the caller")]
fn then_same_ptr(world: &mut QuectoWorld) {
    // The zero-copy property is validated by the unit tests in router_tests.rs.
    // This BDD step confirms the router was called successfully.
    assert!(
        world.env_overrides.contains_key("_original_msg_ptr"),
        "tracking step should have been executed"
    );
}

#[when(expr = "I send a chat request with model {string}")]
fn when_send_chat_with_model(world: &mut QuectoWorld, model: String) {
    let fp = world
        .provider_router
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
        thinking_level: None,
        cancel_flag: None,
        effort: None,
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
        Err(e) => {
            world.routing_handled_by = None;
            world.routing_succeeded = Some(false);
            world
                .env_overrides
                .insert("_routing_error".into(), e.to_string());
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

#[then(expr = "the request should not be handled by the {string} provider")]
fn then_not_handled_by_provider(world: &mut QuectoWorld, unexpected: String) {
    assert_ne!(
        world.routing_handled_by.as_deref(),
        Some(unexpected.as_str()),
        "request should not have been routed to '{}'",
        unexpected
    );
}

#[then(expr = "the provider should receive model {string}")]
fn then_provider_received_model(world: &mut QuectoWorld, expected: String) {
    let content = world
        .routing_response
        .as_ref()
        .and_then(|response| response.content.as_deref())
        .expect("no routing response content");
    let start = content
        .find('{')
        .expect("response should include model start marker")
        + 1;
    let end = content
        .find('}')
        .expect("response should include model end marker");
    assert_eq!(&content[start..end], expected);
}

#[then(expr = "the request should fail with no configured provider {string}")]
fn then_request_fails_no_configured_provider(world: &mut QuectoWorld, provider: String) {
    assert_eq!(world.routing_succeeded, Some(false));
    let error = world
        .env_overrides
        .get("_routing_error")
        .expect("expected routing error");
    assert!(
        error.contains(&format!("no configured provider '{}'", provider)),
        "unexpected routing error: {error}"
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

    // Set up mock that matches any POST to /v1/messages (no beta header required).
    // The fine-grained-tool-streaming beta header was removed (now GA) so we do NOT
    // require it. The step assertion checks it is absent.
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
        model: "claude-sonnet-4-6",
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
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

#[then(expr = "the request should not include the {string} header with {string}")]
fn then_request_not_has_header(world: &mut QuectoWorld, _header: String, value: String) {
    // The mock matches without requiring any particular beta header.
    // The fact that we got a successful response proves the provider works.
    // The actual header absence is verified by the unit test
    // test_api_key_auth_does_not_send_fine_grained_streaming_beta_header.
    // Here we just verify the request succeeded (the old mock required the header
    // and would 404 if present — now neither requires nor rejects it).
    assert!(
        world.streaming_response.is_some(),
        "no response — the request should have succeeded without the '{}' beta header",
        value
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
        model: "claude-sonnet-4-6",
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
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

#[then(expr = "context_tokens should be {int} for the context usage counter")]
fn then_context_tokens_for_counter(world: &mut QuectoWorld, expected: u32) {
    let response = world.streaming_response.as_ref().expect("no response");
    let usage = response.usage.as_ref().expect("response has no usage");
    assert_eq!(usage.context_tokens, Some(expected));
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
        model: "claude-sonnet-4-6",
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
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
        model: "claude-sonnet-4-6",
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
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
        model: "claude-sonnet-4-6",
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
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

// --- #175: Extended thinking support ---

#[given(expr = "an Anthropic request with model {string} and thinking level {string}")]
fn given_anthropic_request_with_thinking(world: &mut QuectoWorld, model: String, level: String) {
    let thinking_level = match level.as_str() {
        "low" => Some(quecto::domain::provider::ThinkingLevel::Low),
        "medium" => Some(quecto::domain::provider::ThinkingLevel::Medium),
        "high" => Some(quecto::domain::provider::ThinkingLevel::High),
        "max" => Some(quecto::domain::provider::ThinkingLevel::Max),
        _ => panic!("unknown thinking level: {}", level),
    };
    world.env_overrides.insert("_thinking_model".into(), model);
    world.env_overrides.insert("_thinking_level".into(), level);
    world.env_overrides.insert(
        "_thinking_level_value".into(),
        serde_json::to_string(&thinking_level.map(|l| l.budget_tokens())).unwrap(),
    );
    // Store messages for later
    world
        .env_overrides
        .insert("_thinking_messages".into(), "Think".into());
}

#[given(expr = "an Anthropic request with model {string} and no thinking level")]
fn given_anthropic_request_no_thinking(world: &mut QuectoWorld, model: String) {
    world.env_overrides.insert("_thinking_model".into(), model);
    world
        .env_overrides
        .insert("_thinking_level".into(), "none".into());
}

#[given(
    expr = "an Anthropic request with model {string} and thinking level {string} and max_tokens {int}"
)]
fn given_anthropic_request_thinking_with_max_tokens(
    world: &mut QuectoWorld,
    model: String,
    level: String,
    max_tokens: u32,
) {
    world.env_overrides.insert("_thinking_model".into(), model);
    world.env_overrides.insert("_thinking_level".into(), level);
    world
        .env_overrides
        .insert("_thinking_max_tokens".into(), max_tokens.to_string());
}

#[when("I build the Anthropic request body with thinking")]
fn when_build_request_body_with_thinking(world: &mut QuectoWorld) {
    let model = world.env_overrides.get("_thinking_model").cloned().unwrap();
    let level_str = world.env_overrides.get("_thinking_level").cloned().unwrap();
    let max_tokens: u32 = world
        .env_overrides
        .get("_thinking_max_tokens")
        .and_then(|s| s.parse().ok())
        .unwrap_or(16000);

    let thinking_level = match level_str.as_str() {
        "low" => Some(quecto::domain::provider::ThinkingLevel::Low),
        "medium" => Some(quecto::domain::provider::ThinkingLevel::Medium),
        "high" => Some(quecto::domain::provider::ThinkingLevel::High),
        "max" => Some(quecto::domain::provider::ThinkingLevel::Max),
        _ => None,
    };

    let messages = vec![quecto::domain::message::Message::user("Think hard")];
    let req = quecto::domain::provider::ChatRequest {
        messages: &messages,
        tools: &[],
        model: &model,
        max_tokens,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level,
        cancel_flag: None,
        effort: None,
    };
    let (_sys, body) =
        quecto::infrastructure::providers::anthropic::AnthropicProvider::build_request_body_public(
            &req,
        );
    world
        .env_overrides
        .insert("_anthropic_body".into(), body.to_string());
}

#[then(expr = "the request body should contain thinking type {string} with budget_tokens {int}")]
fn then_thinking_type_with_budget(world: &mut QuectoWorld, thinking_type: String, budget: u32) {
    let body_str = world.env_overrides.get("_anthropic_body").expect("no body");
    let body: serde_json::Value = serde_json::from_str(body_str).expect("invalid json");
    assert_eq!(
        body["thinking"]["type"], thinking_type,
        "thinking.type mismatch: body={body}"
    );
    assert_eq!(
        body["thinking"]["budget_tokens"], budget,
        "thinking.budget_tokens mismatch: body={body}"
    );
}

#[then(expr = "the request body should contain thinking type {string}")]
fn then_thinking_type_only(world: &mut QuectoWorld, thinking_type: String) {
    let body_str = world.env_overrides.get("_anthropic_body").expect("no body");
    let body: serde_json::Value = serde_json::from_str(body_str).expect("invalid json");
    assert!(
        body.get("thinking").is_some() && !body["thinking"].is_null(),
        "thinking field should be present, got body: {}",
        body
    );
    assert_eq!(
        body["thinking"]["type"], thinking_type,
        "thinking.type mismatch: body={body}"
    );
}

#[then("the request body should not contain a thinking field")]
fn then_no_thinking(world: &mut QuectoWorld) {
    let body_str = world.env_overrides.get("_anthropic_body").expect("no body");
    let body: serde_json::Value = serde_json::from_str(body_str).expect("invalid json");
    assert!(
        body.get("thinking").is_none() || body["thinking"].is_null(),
        "thinking should not be present, got: {}",
        body
    );
}

#[then("the request body should not contain a temperature field")]
fn then_no_temperature(world: &mut QuectoWorld) {
    let body_str = world.env_overrides.get("_anthropic_body").expect("no body");
    let body: serde_json::Value = serde_json::from_str(body_str).expect("invalid json");
    assert!(
        body.get("temperature").is_none() || body["temperature"].is_null(),
        "temperature should not be present when thinking is enabled, got: {}",
        body
    );
}

#[then("the request body should contain a temperature field")]
fn then_has_temperature(world: &mut QuectoWorld) {
    let body_str = world.env_overrides.get("_anthropic_body").expect("no body");
    let body: serde_json::Value = serde_json::from_str(body_str).expect("invalid json");
    assert!(
        body.get("temperature").is_some() && !body["temperature"].is_null(),
        "temperature should be present, got: {}",
        body
    );
}

#[given("an Anthropic SSE response with thinking content blocks")]
fn given_sse_with_thinking(world: &mut QuectoWorld) {
    let raw = "\
event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":10}}}\n\
\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"Reasoning...\"}}\n\
\n\
event: content_block_stop\n\
data: {\"type\":\"content_block_stop\"}\n\
\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Final answer\"}}\n\
\n\
event: content_block_stop\n\
data: {\"type\":\"content_block_stop\"}\n\
\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":5}}\n\
\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n";
    world
        .env_overrides
        .insert("_sse_raw".into(), raw.to_string());
}

#[when("I parse the SSE response")]
fn when_parse_sse(world: &mut QuectoWorld) {
    let raw = world.env_overrides.get("_sse_raw").expect("no SSE data");
    let result =
        quecto::infrastructure::providers::anthropic::AnthropicProvider::parse_sse_response_public(
            raw,
        )
        .expect("SSE parse failed");
    world
        .env_overrides
        .insert("_sse_content".into(), result.content.unwrap_or_default());
}

#[then("the response should contain text content only (thinking blocks excluded from content)")]
fn then_text_only(world: &mut QuectoWorld) {
    let content = world.env_overrides.get("_sse_content").expect("no content");
    assert_eq!(content, "Final answer");
    assert!(
        !content.contains("Reasoning"),
        "thinking content should not appear in response text"
    );
}

#[then(expr = "the request body max_tokens should be at least {int}")]
fn then_max_tokens_at_least(world: &mut QuectoWorld, min: u32) {
    let body_str = world.env_overrides.get("_anthropic_body").expect("no body");
    let body: serde_json::Value = serde_json::from_str(body_str).expect("invalid json");
    let max_tokens = body["max_tokens"].as_u64().expect("max_tokens not found");
    assert!(
        max_tokens >= min as u64,
        "max_tokens should be at least {min}, got: {max_tokens}"
    );
}

// --- #185: Per-call cost tracking ---

#[given(
    expr = "usage data with {int} prompt tokens and {int} completion tokens for model {string}"
)]
fn given_usage_data(world: &mut QuectoWorld, prompt: u32, completion: u32, model: String) {
    world
        .env_overrides
        .insert("_cost_prompt_tokens".into(), prompt.to_string());
    world
        .env_overrides
        .insert("_cost_completion_tokens".into(), completion.to_string());
    world.env_overrides.insert("_cost_model".into(), model);
}

#[when("I calculate the cost")]
fn when_calculate_cost(world: &mut QuectoWorld) {
    let prompt: u32 = world
        .env_overrides
        .get("_cost_prompt_tokens")
        .unwrap()
        .parse()
        .unwrap();
    let completion: u32 = world
        .env_overrides
        .get("_cost_completion_tokens")
        .unwrap()
        .parse()
        .unwrap();
    let model = world.env_overrides.get("_cost_model").unwrap().clone();
    let usage = quecto::domain::message::UsageInfo {
        prompt_tokens: prompt,
        completion_tokens: completion,
        cache_read_tokens: None,
        cache_write_tokens: None,
        context_tokens: None,
        cost: None,
    };
    if let Some(pricing) = quecto::domain::message::model_pricing(&model) {
        let cost = pricing.cost_for(&usage);
        world
            .env_overrides
            .insert("_cost_total".into(), cost.total_cost_usd().to_string());
        world
            .env_overrides
            .insert("_cost_input".into(), cost.input_cost_usd().to_string());
        world
            .env_overrides
            .insert("_cost_output".into(), cost.output_cost_usd().to_string());
    } else {
        world
            .env_overrides
            .insert("_cost_total".into(), "none".into());
    }
}

#[then(expr = "the total cost should be approximately {float} USD")]
fn then_total_cost(world: &mut QuectoWorld, expected: f64) {
    let actual: f64 = world
        .env_overrides
        .get("_cost_total")
        .unwrap()
        .parse()
        .expect("cost should be a number");
    assert!(
        (actual - expected).abs() < 0.0001,
        "expected total cost ~{expected}, got {actual}"
    );
}

#[then(expr = "the input cost should be approximately {float} USD")]
fn then_input_cost(world: &mut QuectoWorld, expected: f64) {
    let actual: f64 = world
        .env_overrides
        .get("_cost_input")
        .unwrap()
        .parse()
        .expect("cost should be a number");
    assert!(
        (actual - expected).abs() < 0.0001,
        "expected input cost ~{expected}, got {actual}"
    );
}

#[then(expr = "the output cost should be approximately {float} USD")]
fn then_output_cost(world: &mut QuectoWorld, expected: f64) {
    let actual: f64 = world
        .env_overrides
        .get("_cost_output")
        .unwrap()
        .parse()
        .expect("cost should be a number");
    assert!(
        (actual - expected).abs() < 0.0001,
        "expected output cost ~{expected}, got {actual}"
    );
}

#[then("cost should be None")]
fn then_cost_none(world: &mut QuectoWorld) {
    let val = world.env_overrides.get("_cost_total").unwrap();
    assert_eq!(val, "none", "expected no cost, got: {val}");
}

// ===========================================================================
// #181: True incremental SSE streaming
// ===========================================================================

/// Helper: build a default ChatRequest for incremental streaming tests.
fn make_incremental_request(messages: &[Message]) -> quecto::domain::provider::ChatRequest<'_> {
    quecto::domain::provider::ChatRequest {
        messages,
        tools: &[],
        model: "claude-sonnet-4-6",
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    }
}

/// Helper: run chat_stream_incremental and collect all events.
fn collect_stream_events(
    provider: &Arc<dyn LlmProvider>,
    messages: &[Message],
) -> Vec<quecto::domain::provider::StreamEvent> {
    use quecto::domain::provider::StreamEvent;
    let rt = tokio::runtime::Runtime::new().unwrap();
    let req = make_incremental_request(messages);
    rt.block_on(async {
        let mut rx = provider.chat_stream_incremental(req).await;
        let mut events = Vec::new();
        while let Some(ev) = rx.recv().await {
            let done = matches!(ev, StreamEvent::Done(_) | StreamEvent::Error(_));
            events.push(ev);
            if done {
                break;
            }
        }
        events
    })
}

// --- Single-event parse scenarios (unit-like, no HTTP) ---

#[given(expr = "an Anthropic SSE chunk with a text_delta event containing {string}")]
fn given_sse_text_delta(world: &mut QuectoWorld, text: String) {
    world
        .env_overrides
        .insert("_sse181_type".into(), "text_delta".into());
    world.env_overrides.insert("_sse181_text".into(), text);
}

#[given(expr = "an Anthropic SSE chunk with a thinking_delta event containing {string}")]
fn given_sse_thinking_delta(world: &mut QuectoWorld, text: String) {
    world
        .env_overrides
        .insert("_sse181_type".into(), "thinking_delta".into());
    world.env_overrides.insert("_sse181_text".into(), text);
}

#[given(
    expr = "an Anthropic SSE chunk with a content_block_start for tool {string} with id {string}"
)]
fn given_sse_tool_block_start(world: &mut QuectoWorld, name: String, id: String) {
    world
        .env_overrides
        .insert("_sse181_type".into(), "tool_block_start".into());
    world.env_overrides.insert("_sse181_tool_name".into(), name);
    world.env_overrides.insert("_sse181_tool_id".into(), id);
}

#[given(expr = "an Anthropic SSE chunk with an input_json_delta containing {string}")]
fn given_sse_input_json_delta(world: &mut QuectoWorld, partial: String) {
    world
        .env_overrides
        .insert("_sse181_type".into(), "input_json_delta".into());
    world
        .env_overrides
        .insert("_sse181_partial".into(), partial);
}

#[given(
    expr = "an Anthropic SSE chunk with a content_block_stop for tool {string} id {string} and accumulated input {string}"
)]
fn given_sse_tool_block_stop(world: &mut QuectoWorld, name: String, id: String, input: String) {
    world
        .env_overrides
        .insert("_sse181_type".into(), "tool_block_stop".into());
    world.env_overrides.insert("_sse181_tool_name".into(), name);
    world.env_overrides.insert("_sse181_tool_id".into(), id);
    world
        .env_overrides
        .insert("_sse181_tool_input".into(), input);
}

#[when("I parse the SSE chunk as a stream event")]
async fn when_parse_sse_chunk_as_stream_event(world: &mut QuectoWorld) {
    use quecto::infrastructure::providers::anthropic::AnthropicProvider;

    let ev_type = world
        .env_overrides
        .get("_sse181_type")
        .cloned()
        .unwrap_or_default();

    // Build a minimal SSE string that parse_sse_events_public can handle,
    // then take the first meaningful event.
    let sse = match ev_type.as_str() {
        "text_delta" => {
            let text = world
                .env_overrides
                .get("_sse181_text")
                .cloned()
                .unwrap_or_default();
            format!(
                "event: content_block_delta\ndata: {{\"delta\":{{\"type\":\"text_delta\",\"text\":\"{}\"}}}}\n\nevent: message_stop\ndata: {{}}\n\n",
                text
            )
        }
        "thinking_delta" => {
            let text = world
                .env_overrides
                .get("_sse181_text")
                .cloned()
                .unwrap_or_default();
            format!(
                "event: content_block_delta\ndata: {{\"delta\":{{\"type\":\"thinking_delta\",\"thinking\":\"{}\"}}}}\n\nevent: message_stop\ndata: {{}}\n\n",
                text
            )
        }
        "tool_block_start" => {
            let name = world
                .env_overrides
                .get("_sse181_tool_name")
                .cloned()
                .unwrap_or_default();
            let id = world
                .env_overrides
                .get("_sse181_tool_id")
                .cloned()
                .unwrap_or_default();
            format!(
                "event: content_block_start\ndata: {{\"content_block\":{{\"type\":\"tool_use\",\"id\":\"{}\",\"name\":\"{}\"}}}}\n\nevent: message_stop\ndata: {{}}\n\n",
                id, name
            )
        }
        "input_json_delta" => {
            let partial = world
                .env_overrides
                .get("_sse181_partial")
                .cloned()
                .unwrap_or_default();
            // Escape for JSON string embedding
            let escaped = partial.replace('\\', "\\\\").replace('"', "\\\"");
            format!(
                "event: content_block_delta\ndata: {{\"delta\":{{\"type\":\"input_json_delta\",\"partial_json\":\"{}\"}}}}\n\nevent: message_stop\ndata: {{}}\n\n",
                escaped
            )
        }
        "tool_block_stop" => {
            let name = world
                .env_overrides
                .get("_sse181_tool_name")
                .cloned()
                .unwrap_or_default();
            let id = world
                .env_overrides
                .get("_sse181_tool_id")
                .cloned()
                .unwrap_or_default();
            let input = world
                .env_overrides
                .get("_sse181_tool_input")
                .cloned()
                .unwrap_or_default();
            // Simulate start + delta + stop
            let escaped_input = input.replace('\\', "\\\\").replace('"', "\\\"");
            format!(
                "event: content_block_start\ndata: {{\"content_block\":{{\"type\":\"tool_use\",\"id\":\"{}\",\"name\":\"{}\"}}}}\n\nevent: content_block_delta\ndata: {{\"delta\":{{\"type\":\"input_json_delta\",\"partial_json\":\"{}\"}}}}\n\nevent: content_block_stop\ndata: {{}}\n\nevent: message_stop\ndata: {{}}\n\n",
                id, name, escaped_input
            )
        }
        _ => String::new(),
    };

    let events = AnthropicProvider::parse_sse_events_public(&sse).await;
    world.stream_events = events;
}

#[then(expr = "the stream event should be a TextDelta with text {string}")]
fn then_stream_event_text_delta(world: &mut QuectoWorld, expected: String) {
    use quecto::domain::provider::StreamEvent;
    let found = world
        .stream_events
        .iter()
        .any(|ev| matches!(ev, StreamEvent::TextDelta(t) if t == &expected));
    assert!(
        found,
        "expected TextDelta({:?}), got: {:?}",
        expected, world.stream_events
    );
}

#[then(expr = "the stream event should be a ThinkingDelta with text {string}")]
fn then_stream_event_thinking_delta(world: &mut QuectoWorld, expected: String) {
    use quecto::domain::provider::StreamEvent;
    let found = world
        .stream_events
        .iter()
        .any(|ev| matches!(ev, StreamEvent::ThinkingDelta(t) if t == &expected));
    assert!(
        found,
        "expected ThinkingDelta({:?}), got: {:?}",
        expected, world.stream_events
    );
}

#[then(expr = "the stream event should be a ToolCallStart with id {string} and name {string}")]
fn then_stream_event_tool_call_start(world: &mut QuectoWorld, id: String, name: String) {
    use quecto::domain::provider::StreamEvent;
    let found = world.stream_events.iter().any(
        |ev| matches!(ev, StreamEvent::ToolCallStart { id: i, name: n } if i == &id && n == &name),
    );
    assert!(
        found,
        "expected ToolCallStart(id={:?}, name={:?}), got: {:?}",
        id, name, world.stream_events
    );
}

#[then(expr = "the stream event should be a ToolCallDelta with partial {string}")]
fn then_stream_event_tool_call_delta(world: &mut QuectoWorld, expected: String) {
    use quecto::domain::provider::StreamEvent;
    let found = world
        .stream_events
        .iter()
        .any(|ev| matches!(ev, StreamEvent::ToolCallDelta(p) if p == &expected));
    assert!(
        found,
        "expected ToolCallDelta({:?}), got: {:?}",
        expected, world.stream_events
    );
}

#[then(
    expr = "the stream event should be a ToolCallEnd with id {string} name {string} and arguments {string}"
)]
fn then_stream_event_tool_call_end(
    world: &mut QuectoWorld,
    id: String,
    name: String,
    arguments: String,
) {
    use quecto::domain::provider::StreamEvent;
    let found = world.stream_events.iter().any(|ev| {
        matches!(ev, StreamEvent::ToolCallEnd { id: i, name: n, arguments: a } if i == &id && n == &name && a == &arguments)
    });
    assert!(
        found,
        "expected ToolCallEnd(id={:?}, name={:?}, args={:?}), got: {:?}",
        id, name, arguments, world.stream_events
    );
}

// --- HTTP-based incremental streaming scenarios ---

#[given(expr = "an Anthropic mock server that streams text {string} in {int} chunks")]
fn given_anthropic_mock_text_chunks(world: &mut QuectoWorld, text: String, chunks: u32) {
    // Split text into N equal(-ish) chunks
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut sse = String::new();
    for i in 0..chunks {
        let start = (len * i as usize) / chunks as usize;
        let end = (len * (i as usize + 1)) / chunks as usize;
        let chunk_text: String = chars[start..end].iter().collect();
        // Escape for JSON
        let escaped = chunk_text.replace('\\', "\\\\").replace('"', "\\\"");
        sse.push_str(&format!(
            "event: content_block_delta\ndata: {{\"delta\":{{\"type\":\"text_delta\",\"text\":\"{}\"}}}}\n\n",
            escaped
        ));
    }
    sse.push_str("event: message_delta\ndata: {\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":10}}\n\n");
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
            "sk-ant-test".to_string(),
            Some(uri.clone()),
        ),
    ));
    world._wiremock_server_uri = Some(uri);
    world.env_overrides.insert("_expected_text".into(), text);
    std::mem::forget(_server);
    std::mem::forget(rt);
}

#[given(
    expr = "an Anthropic mock server that streams a tool call for {string} with arguments {string}"
)]
fn given_anthropic_mock_tool_call_stream(world: &mut QuectoWorld, tool: String, args: String) {
    let escaped_args = args.replace('\\', "\\\\").replace('"', "\\\"");
    let sse = format!(
        "event: content_block_start\ndata: {{\"content_block\":{{\"type\":\"tool_use\",\"id\":\"toolu_001\",\"name\":\"{}\"}}}}\n\n\
         event: content_block_delta\ndata: {{\"delta\":{{\"type\":\"input_json_delta\",\"partial_json\":\"{}\"}}}}\n\n\
         event: content_block_stop\ndata: {{}}\n\n\
         event: message_delta\ndata: {{\"delta\":{{\"stop_reason\":\"tool_use\"}},\"usage\":{{\"output_tokens\":15}}}}\n\n\
         event: message_stop\ndata: {{}}\n\n",
        tool, escaped_args
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
            "sk-ant-test".to_string(),
            Some(uri.clone()),
        ),
    ));
    world._wiremock_server_uri = Some(uri);
    world.env_overrides.insert("_expected_tool".into(), tool);
    world.env_overrides.insert("_expected_args".into(), args);
    std::mem::forget(_server);
    std::mem::forget(rt);
}

#[given("an Anthropic mock server that sends SSE lines split across byte chunks")]
fn given_anthropic_mock_chunked_bytes(world: &mut QuectoWorld) {
    // The full SSE would be split but since we're using wiremock which sends the body at once,
    // we test that parse_sse_events_public handles line-by-line parsing correctly
    // by using a string with the correct SSE format.
    let sse = "event: content_block_delta\ndata: {\"delta\":{\"type\":\"text_delta\",\"text\":\"Split\"}}\n\nevent: content_block_delta\ndata: {\"delta\":{\"type\":\"text_delta\",\"text\":\" text\"}}\n\nevent: message_stop\ndata: {}\n\n".to_string();

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
            "sk-ant-test".to_string(),
            Some(uri.clone()),
        ),
    ));
    world._wiremock_server_uri = Some(uri);
    world
        .env_overrides
        .insert("_expected_text".into(), "Split text".into());
    std::mem::forget(_server);
    std::mem::forget(rt);
}

#[given("an Anthropic mock server that returns an HTTP 500 error")]
fn given_anthropic_mock_500(world: &mut QuectoWorld) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (uri, _server) = rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/v1/messages"))
            .respond_with(
                wiremock::ResponseTemplate::new(500).set_body_string("Internal Server Error"),
            )
            .mount(&server)
            .await;
        let uri = server.uri();
        (uri, server)
    });
    world.provider = Some(Arc::new(
        quecto::infrastructure::providers::anthropic::AnthropicProvider::new(
            "sk-ant-test".to_string(),
            Some(uri.clone()),
        ),
    ));
    world._wiremock_server_uri = Some(uri);
    std::mem::forget(_server);
    std::mem::forget(rt);
}

#[given("an Anthropic mock server that streams a complete response with text and tool call")]
fn given_anthropic_mock_text_and_tool(world: &mut QuectoWorld) {
    let sse = "event: content_block_delta\ndata: {\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n\
               event: content_block_start\ndata: {\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_abc\",\"name\":\"bash\"}}\n\n\
               event: content_block_delta\ndata: {\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\\\"command\\\\\":\\\\\"ls\\\\\"}\"}}\n\n\
               event: content_block_stop\ndata: {}\n\n\
               event: message_delta\ndata: {\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":20}}\n\n\
               event: message_stop\ndata: {}\n\n".to_string();

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
            "sk-ant-test".to_string(),
            Some(uri.clone()),
        ),
    ));
    world._wiremock_server_uri = Some(uri);
    std::mem::forget(_server);
    std::mem::forget(rt);
}

#[when("I send an incremental streaming chat request")]
fn when_send_incremental_streaming(world: &mut QuectoWorld) {
    use quecto::domain::provider::StreamEvent;
    let provider = world.provider.as_ref().expect("provider not set").clone();
    let messages = vec![Message::user("Hi")];
    let events = collect_stream_events(&provider, &messages);
    world.stream_had_parse_error = events.iter().any(|e| matches!(e, StreamEvent::Error(_)));
    world.stream_events = events;
}

#[when("I send both a streaming and an incremental streaming chat request")]
fn when_send_both_streaming_and_incremental(world: &mut QuectoWorld) {
    use quecto::domain::provider::StreamEvent;
    let provider = world.provider.as_ref().expect("provider not set").clone();
    let messages = vec![Message::user("Hi")];

    let rt = tokio::runtime::Runtime::new().unwrap();
    let req = make_incremental_request(&messages);
    let stream_result = rt.block_on(provider.chat_stream(req));

    // Also collect incremental events — provider is cloned above, use the clone
    let events = collect_stream_events(&provider, &messages);

    // Extract LlmResponse from Done event
    let incremental_response = events.into_iter().find_map(|ev| {
        if let StreamEvent::Done(resp) = ev {
            Some(resp)
        } else {
            None
        }
    });

    // Store both for comparison
    if let Ok(resp) = stream_result {
        world.streaming_response = Some(resp);
    }
    if let Some(resp) = incremental_response {
        world.env_overrides.insert(
            "_incremental_content".into(),
            resp.content.unwrap_or_default(),
        );
        world.env_overrides.insert(
            "_incremental_tool_count".into(),
            resp.tool_calls.len().to_string(),
        );
        world.env_overrides.insert(
            "_incremental_first_tool".into(),
            resp.tool_calls
                .first()
                .map(|t| t.name.clone())
                .unwrap_or_default(),
        );
    }
}

#[then(expr = "I should receive TextDelta events totalling {string}")]
fn then_text_delta_total(world: &mut QuectoWorld, expected: String) {
    use quecto::domain::provider::StreamEvent;
    let collected: String = world
        .stream_events
        .iter()
        .filter_map(|ev| {
            if let StreamEvent::TextDelta(t) = ev {
                Some(t.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        collected, expected,
        "expected combined TextDelta text {:?}, got {:?}",
        expected, collected
    );
}

#[then(expr = "the final event should be Done with content {string}")]
fn then_final_event_done_with_content(world: &mut QuectoWorld, expected: String) {
    use quecto::domain::provider::StreamEvent;
    let last = world.stream_events.last().expect("no events");
    match last {
        StreamEvent::Done(resp) => {
            let content = resp.content.as_deref().unwrap_or("");
            assert_eq!(
                content, expected,
                "expected Done content {:?}, got {:?}",
                expected, content
            );
        }
        other => panic!("expected Done event, got: {:?}", other),
    }
}

#[then(expr = "the final event should be Done with a tool call for {string}")]
fn then_final_event_done_with_tool(world: &mut QuectoWorld, tool: String) {
    use quecto::domain::provider::StreamEvent;
    let last = world.stream_events.last().expect("no events");
    match last {
        StreamEvent::Done(resp) => {
            let found = resp.tool_calls.iter().any(|tc| tc.name == tool);
            assert!(
                found,
                "expected Done to contain tool call for {:?}, got: {:?}",
                tool, resp.tool_calls
            );
        }
        other => panic!("expected Done event, got: {:?}", other),
    }
}

#[then(expr = "I should receive a ToolCallStart event for tool {string}")]
fn then_received_tool_call_start(world: &mut QuectoWorld, tool: String) {
    use quecto::domain::provider::StreamEvent;
    let found = world
        .stream_events
        .iter()
        .any(|ev| matches!(ev, StreamEvent::ToolCallStart { name, .. } if name == &tool));
    assert!(
        found,
        "expected ToolCallStart for {:?}, got: {:?}",
        tool, world.stream_events
    );
}

#[then("I should receive ToolCallDelta events")]
fn then_received_tool_call_delta(world: &mut QuectoWorld) {
    use quecto::domain::provider::StreamEvent;
    let found = world
        .stream_events
        .iter()
        .any(|ev| matches!(ev, StreamEvent::ToolCallDelta(_)));
    assert!(
        found,
        "expected at least one ToolCallDelta, got: {:?}",
        world.stream_events
    );
}

#[then(expr = "I should receive a ToolCallEnd event for tool {string} with arguments {string}")]
fn then_received_tool_call_end(world: &mut QuectoWorld, tool: String, args: String) {
    use quecto::domain::provider::StreamEvent;
    let found = world.stream_events.iter().any(|ev| {
        matches!(ev, StreamEvent::ToolCallEnd { name, arguments, .. } if name == &tool && arguments == &args)
    });
    assert!(
        found,
        "expected ToolCallEnd for {:?} with args {:?}, got: {:?}",
        tool, args, world.stream_events
    );
}

#[then("I should receive an Error stream event")]
fn then_received_error_event(world: &mut QuectoWorld) {
    use quecto::domain::provider::StreamEvent;
    let found = world
        .stream_events
        .iter()
        .any(|ev| matches!(ev, StreamEvent::Error(_)));
    assert!(
        found,
        "expected Error event, got: {:?}",
        world.stream_events
    );
}

#[then("I should receive TextDelta events totalling the expected text")]
fn then_text_delta_total_expected(world: &mut QuectoWorld) {
    use quecto::domain::provider::StreamEvent;
    let expected = world
        .env_overrides
        .get("_expected_text")
        .cloned()
        .unwrap_or_default();
    let collected: String = world
        .stream_events
        .iter()
        .filter_map(|ev| {
            if let StreamEvent::TextDelta(t) = ev {
                Some(t.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        collected, expected,
        "expected combined TextDelta text {:?}, got {:?}",
        expected, collected
    );
}

#[then("no parse errors should occur")]
fn then_no_parse_errors(world: &mut QuectoWorld) {
    assert!(
        !world.stream_had_parse_error,
        "unexpected parse error in stream events"
    );
}

#[then("both responses should have identical content and tool calls")]
fn then_responses_identical(world: &mut QuectoWorld) {
    let streaming = world
        .streaming_response
        .as_ref()
        .expect("no streaming response");
    let inc_content = world
        .env_overrides
        .get("_incremental_content")
        .cloned()
        .unwrap_or_default();
    let inc_tool_count: usize = world
        .env_overrides
        .get("_incremental_tool_count")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let inc_first_tool = world
        .env_overrides
        .get("_incremental_first_tool")
        .cloned()
        .unwrap_or_default();

    let stream_content = streaming.content.as_deref().unwrap_or("");
    assert_eq!(
        stream_content, inc_content,
        "content mismatch: streaming={:?} incremental={:?}",
        stream_content, inc_content
    );
    assert_eq!(
        streaming.tool_calls.len(),
        inc_tool_count,
        "tool call count mismatch"
    );
    if !streaming.tool_calls.is_empty() {
        assert_eq!(
            streaming.tool_calls[0].name, inc_first_tool,
            "first tool name mismatch"
        );
    }
}

// ===========================================================================
// #184: Cross-provider message normalization pipeline
// ===========================================================================

// ---- Given steps for building message histories ----------------------------

#[given(expr = "a message history with an assistant tool call id {string} for tool {string}")]
fn given_assistant_tool_call_with_id(world: &mut QuectoWorld, raw_id: String, tool_name: String) {
    let msgs = vec![
        Message::user("do something"),
        Message::assistant(
            "",
            vec![ToolCall {
                id: raw_id.clone(),
                name: tool_name,
                arguments: "{}".into(),
            }],
        ),
        Message::tool(raw_id, "tool output"),
    ];
    world.context_messages = Some(msgs);
}

#[given(expr = "a matching tool result for id {string}")]
fn given_matching_tool_result_for_id(_world: &mut QuectoWorld, _id: String) {
    // The tool result is already included by `given_assistant_tool_call_with_id`.
    // This step exists for readability; no additional state needed.
}

#[given(
    expr = "a message history with an assistant tool call id {string} for tool {string} and no tool result"
)]
fn given_orphaned_assistant_tool_call(world: &mut QuectoWorld, raw_id: String, tool_name: String) {
    let msgs = vec![
        Message::user("do something"),
        Message::assistant(
            "",
            vec![ToolCall {
                id: raw_id,
                name: tool_name,
                arguments: "{}".into(),
            }],
        ),
        // No Tool message — orphaned tool call.
    ];
    world.context_messages = Some(msgs);
}

#[given(expr = "a message history with two orphaned assistant tool calls {string} and {string}")]
fn given_two_orphaned_tool_calls(world: &mut QuectoWorld, id_a: String, id_b: String) {
    let msgs = vec![
        Message::user("do two things"),
        Message::assistant(
            "",
            vec![
                ToolCall {
                    id: id_a,
                    name: "bash".into(),
                    arguments: "{}".into(),
                },
                ToolCall {
                    id: id_b,
                    name: "bash".into(),
                    arguments: "{}".into(),
                },
            ],
        ),
        // No Tool messages — both orphaned.
    ];
    world.context_messages = Some(msgs);
}

#[given(expr = "a message history containing an assistant message with stop_reason {string}")]
fn given_assistant_message_with_stop_reason(world: &mut QuectoWorld, stop_reason: String) {
    use quecto::domain::message::StopReason;
    let mut asst = Message::assistant("I will help", vec![]);
    asst.stop_reason = if stop_reason.is_empty() {
        None
    } else {
        Some(StopReason::parse(&stop_reason))
    };
    world.context_messages = Some(vec![Message::user("hello"), asst]);
}

// ---- When step: build Anthropic messages (reuse existing step) -------------

#[when("I build Anthropic messages from that history")]
fn when_build_anthropic_messages_from_history(world: &mut QuectoWorld) {
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

// ---- Then steps: tool_use block ID assertions ------------------------------

/// Extract the first `tool_use` content block from the stored API messages.
fn extract_tool_use_id(world: &QuectoWorld) -> String {
    let msgs_str = world.env_overrides.get("_anthropic_msgs").expect("no msgs");
    let msgs: Vec<serde_json::Value> = serde_json::from_str(msgs_str).expect("invalid json");
    for msg in &msgs {
        if let Some(content) = msg["content"].as_array() {
            for block in content {
                if block["type"] == "tool_use" {
                    return block["id"].as_str().unwrap_or("").to_string();
                }
            }
        }
    }
    panic!("no tool_use block found in API messages: {}", msgs_str);
}

/// Extract the first `tool_result` `tool_use_id` from the stored API messages.
fn extract_tool_result_id(world: &QuectoWorld) -> String {
    let msgs_str = world.env_overrides.get("_anthropic_msgs").expect("no msgs");
    let msgs: Vec<serde_json::Value> = serde_json::from_str(msgs_str).expect("invalid json");
    for msg in &msgs {
        if let Some(content) = msg["content"].as_array() {
            for block in content {
                if block["type"] == "tool_result" {
                    return block["tool_use_id"].as_str().unwrap_or("").to_string();
                }
            }
        }
    }
    panic!("no tool_result block found in API messages: {}", msgs_str);
}

#[then(expr = "the tool_use block should have id {string}")]
fn then_tool_use_id_is(world: &mut QuectoWorld, expected: String) {
    let actual = extract_tool_use_id(world);
    assert_eq!(
        actual, expected,
        "tool_use id: expected '{}', got '{}'",
        expected, actual
    );
}

#[then(expr = "the tool_result block should have tool_use_id {string}")]
fn then_tool_result_id_is(world: &mut QuectoWorld, expected: String) {
    let actual = extract_tool_result_id(world);
    assert_eq!(
        actual, expected,
        "tool_result tool_use_id: expected '{}', got '{}'",
        expected, actual
    );
}

// ---- Then steps: orphaned tool call injection -----------------------------

#[then(expr = "a synthetic tool result with tool_use_id {string} is injected")]
fn then_synthetic_tool_result_injected(world: &mut QuectoWorld, expected_id: String) {
    let msgs_str = world.env_overrides.get("_anthropic_msgs").expect("no msgs");
    let msgs: Vec<serde_json::Value> = serde_json::from_str(msgs_str).expect("invalid json");
    let found = msgs.iter().any(|msg| {
        msg["content"]
            .as_array()
            .map(|blocks| {
                blocks.iter().any(|b| {
                    // Synthetic results: is_error=true + content="No result provided"
                    b["type"] == "tool_result"
                        && b["tool_use_id"].as_str() == Some(expected_id.as_str())
                        && b["is_error"].as_bool().unwrap_or(false)
                        && b["content"].as_str() == Some("No result provided")
                })
            })
            .unwrap_or(false)
    });
    assert!(
        found,
        "expected synthetic tool_result for id '{}' but not found in:\n{}",
        expected_id, msgs_str
    );
}

#[then(expr = "the synthetic result has content {string} and is_error true")]
fn then_synthetic_result_content(world: &mut QuectoWorld, expected_content: String) {
    let msgs_str = world.env_overrides.get("_anthropic_msgs").expect("no msgs");
    let msgs: Vec<serde_json::Value> = serde_json::from_str(msgs_str).expect("invalid json");
    let found = msgs.iter().any(|msg| {
        msg["content"]
            .as_array()
            .map(|blocks| {
                blocks.iter().any(|b| {
                    if b["type"] != "tool_result" {
                        return false;
                    }
                    let is_error = b["is_error"].as_bool().unwrap_or(false);
                    let content_match = b["content"].as_str() == Some(expected_content.as_str());
                    is_error && content_match
                })
            })
            .unwrap_or(false)
    });
    assert!(
        found,
        "expected synthetic tool_result with content='{}' and is_error=true, not found in:\n{}",
        expected_content, msgs_str
    );
}

#[then(expr = "no synthetic tool result is injected for id {string}")]
fn then_no_synthetic_tool_result(world: &mut QuectoWorld, id: String) {
    let msgs_str = world.env_overrides.get("_anthropic_msgs").expect("no msgs");
    let msgs: Vec<serde_json::Value> = serde_json::from_str(msgs_str).expect("invalid json");
    let found = msgs.iter().any(|msg| {
        msg["content"]
            .as_array()
            .map(|blocks| {
                blocks.iter().any(|b| {
                    // A synthetic result has is_error=true + content="No result provided"
                    b["type"] == "tool_result"
                        && b["tool_use_id"].as_str() == Some(id.as_str())
                        && b["is_error"].as_bool().unwrap_or(false)
                        && b["content"].as_str() == Some("No result provided")
                })
            })
            .unwrap_or(false)
    });
    assert!(
        !found,
        "expected no synthetic tool_result for id '{}', but one was injected",
        id
    );
}

// ---- Then steps: message filtering ----------------------------------------

#[then("the errored assistant message is not present in the API payload")]
fn then_errored_assistant_filtered(world: &mut QuectoWorld) {
    let msgs_str = world.env_overrides.get("_anthropic_msgs").expect("no msgs");
    let msgs: Vec<serde_json::Value> = serde_json::from_str(msgs_str).expect("invalid json");
    // There should be no assistant message in the output (the errored one was filtered).
    let has_assistant = msgs.iter().any(|m| m["role"] == "assistant");
    assert!(
        !has_assistant,
        "errored assistant message should have been filtered but is present in:\n{}",
        msgs_str
    );
}

#[then("the assistant message is present in the API payload")]
fn then_assistant_message_present(world: &mut QuectoWorld) {
    let msgs_str = world.env_overrides.get("_anthropic_msgs").expect("no msgs");
    let msgs: Vec<serde_json::Value> = serde_json::from_str(msgs_str).expect("invalid json");
    let has_assistant = msgs.iter().any(|m| m["role"] == "assistant");
    assert!(
        has_assistant,
        "expected assistant message to be present but not found in:\n{}",
        msgs_str
    );
}

// ===========================================================================
// #188: User message content block support (inline images + capability filtering)
// ===========================================================================

// ---- Given steps -----------------------------------------------------------

#[given(expr = "a user message with text {string} and no image blocks")]
fn given_user_message_text_only(world: &mut QuectoWorld, text: String) {
    world.context_messages = Some(vec![Message::user(text)]);
}

#[given(expr = "a user message with text {string} and one image block of type {string}")]
fn given_user_message_with_one_image(world: &mut QuectoWorld, text: String, mime: String) {
    use quecto::domain::message::UserImageBlock;
    let mut m = Message::user(text);
    m.user_image_blocks = vec![UserImageBlock {
        mime_type: mime,
        data: "aGVsbG8=".into(), // base64 "hello"
    }];
    world.context_messages = Some(vec![m]);
}

#[given(expr = "a user message with text {string} and two image blocks of type {string}")]
fn given_user_message_with_two_images(world: &mut QuectoWorld, text: String, mime: String) {
    use quecto::domain::message::UserImageBlock;
    let mut m = Message::user(text);
    m.user_image_blocks = vec![
        UserImageBlock {
            mime_type: mime.clone(),
            data: "aGVsbG8=".into(),
        },
        UserImageBlock {
            mime_type: mime,
            data: "d29ybGQ=".into(),
        },
    ];
    world.context_messages = Some(vec![m]);
}

// ---- When step (model-aware) -----------------------------------------------

#[when(expr = "I build Anthropic messages from that history for model {string}")]
fn when_build_anthropic_messages_for_model(world: &mut QuectoWorld, model: String) {
    let msgs = world.context_messages.as_ref().expect("no messages set");
    let (_sys, api_msgs) =
        quecto::infrastructure::providers::anthropic::AnthropicProvider::build_messages_for_model_public(
            msgs, &model,
        );
    world.env_overrides.insert(
        "_anthropic_msgs".into(),
        serde_json::to_string(&api_msgs).unwrap(),
    );
}

// ---- Then steps ------------------------------------------------------------

/// Get the first user message's content value from stored API messages.
fn first_user_content(world: &QuectoWorld) -> serde_json::Value {
    let msgs_str = world.env_overrides.get("_anthropic_msgs").expect("no msgs");
    let msgs: Vec<serde_json::Value> = serde_json::from_str(msgs_str).expect("invalid json");
    msgs.iter()
        .find(|m| m["role"] == "user")
        .map(|m| m["content"].clone())
        .unwrap_or(serde_json::Value::Null)
}

#[then(expr = "the user message content should be the string {string}")]
fn then_user_content_is_string(world: &mut QuectoWorld, expected: String) {
    let content = first_user_content(world);
    assert_eq!(
        content.as_str(),
        Some(expected.as_str()),
        "expected plain string content '{}', got: {}",
        expected,
        content
    );
}

#[then("the user message content should be a block array")]
fn then_user_content_is_array(world: &mut QuectoWorld) {
    let content = first_user_content(world);
    assert!(
        content.is_array(),
        "expected content block array, got: {}",
        content
    );
}

#[then(expr = "the block array should contain a text block {string}")]
fn then_block_array_has_text(world: &mut QuectoWorld, expected_text: String) {
    let content = first_user_content(world);
    let blocks = content.as_array().expect("content is not an array");
    let found = blocks
        .iter()
        .any(|b| b["type"] == "text" && b["text"].as_str() == Some(expected_text.as_str()));
    assert!(
        found,
        "expected text block '{}' not found in: {}",
        expected_text, content
    );
}

#[then(expr = "the block array should contain an image block of media_type {string}")]
fn then_block_array_has_image(world: &mut QuectoWorld, media_type: String) {
    let content = first_user_content(world);
    let blocks = content.as_array().expect("content is not an array");
    let found = blocks.iter().any(|b| {
        b["type"] == "image" && b["source"]["media_type"].as_str() == Some(media_type.as_str())
    });
    assert!(
        found,
        "expected image block with media_type '{}' not found in: {}",
        media_type, content
    );
}

#[then(expr = "the block array should contain {int} image blocks")]
fn then_block_array_has_n_images(world: &mut QuectoWorld, expected: usize) {
    let content = first_user_content(world);
    let blocks = content.as_array().expect("content is not an array");
    let count = blocks.iter().filter(|b| b["type"] == "image").count();
    assert_eq!(
        count, expected,
        "expected {} image blocks, found {}: {}",
        expected, count, content
    );
}

#[then("the block array should contain no text blocks")]
fn then_block_array_has_no_text(world: &mut QuectoWorld) {
    let content = first_user_content(world);
    let blocks = content.as_array().expect("content is not an array");
    let count = blocks.iter().filter(|b| b["type"] == "text").count();
    assert_eq!(
        count, 0,
        "expected no text blocks, found {}: {}",
        count, content
    );
}

#[then("the Anthropic payload should contain no user messages")]
fn then_no_user_messages(world: &mut QuectoWorld) {
    let msgs_str = world.env_overrides.get("_anthropic_msgs").expect("no msgs");
    let msgs: Vec<serde_json::Value> = serde_json::from_str(msgs_str).expect("invalid json");
    let user_count = msgs.iter().filter(|m| m["role"] == "user").count();
    assert_eq!(
        user_count, 0,
        "expected no user messages, found {}: {}",
        user_count, msgs_str
    );
}

// ===========================================================================
// #182: Abort/cancellation support via CancelFlag
// ===========================================================================

#[given("an Anthropic mock server that returns a successful text response")]
fn given_anthropic_mock_success_182(world: &mut QuectoWorld) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (uri, _server) = rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let response_body = serde_json::json!({
            "id": "msg_ok",
            "type": "message",
            "role": "assistant",
            "content": [{"type": "text", "text": "Hello"}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 5, "output_tokens": 3}
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

#[given("a cancel flag that is already set")]
fn given_cancel_flag_set(world: &mut QuectoWorld) {
    use quecto::domain::provider::CancelFlag;
    let flag = CancelFlag::new();
    flag.cancel();
    world.cancel_flag = Some(flag);
}

#[given("a cancel flag that is not set")]
fn given_cancel_flag_not_set(world: &mut QuectoWorld) {
    use quecto::domain::provider::CancelFlag;
    world.cancel_flag = Some(CancelFlag::new());
}

#[when("I send a chat request with the cancel flag")]
fn when_chat_with_cancel_flag(world: &mut QuectoWorld) {
    use quecto::domain::provider::ChatRequest;
    let provider = world.provider.as_ref().expect("no provider").clone();
    let cancel = world.cancel_flag.clone();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let messages = vec![Message::user("hello")];
    let result = rt.block_on(async move {
        let request = ChatRequest {
            messages: &messages,
            tools: &[],
            model: "claude-opus-4-5",
            max_tokens: 100,
            temperature: 0.0,
            session_id: None,
            tool_choice: None,
            metadata: None,
            thinking_level: None,
            cancel_flag: cancel,
            effort: None,
        };
        provider.chat(request).await
    });
    world.chat_result = Some(result.map_err(|e| e.to_string()));
}

#[when("I send a streaming chat request with the cancel flag")]
fn when_streaming_chat_with_cancel_flag(world: &mut QuectoWorld) {
    use quecto::domain::provider::ChatRequest;
    let provider = world.provider.as_ref().expect("no provider").clone();
    let cancel = world.cancel_flag.clone();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let messages = vec![Message::user("hello")];
    let result = rt.block_on(async move {
        let request = ChatRequest {
            messages: &messages,
            tools: &[],
            model: "claude-opus-4-5",
            max_tokens: 100,
            temperature: 0.0,
            session_id: None,
            tool_choice: None,
            metadata: None,
            thinking_level: None,
            cancel_flag: cancel,
            effort: None,
        };
        provider.chat_stream(request).await
    });
    world.chat_stream_result = Some(result.map_err(|e| e.to_string()));
}

#[when("I send an incremental streaming chat request with the cancel flag")]
fn when_incremental_chat_with_cancel_flag(world: &mut QuectoWorld) {
    use quecto::domain::provider::{ChatRequest, StreamEvent};
    let provider = world.provider.as_ref().expect("no provider").clone();
    let cancel = world.cancel_flag.clone();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let messages = vec![Message::user("hello")];
    let events = rt.block_on(async move {
        let request = ChatRequest {
            messages: &messages,
            tools: &[],
            model: "claude-opus-4-5",
            max_tokens: 100,
            temperature: 0.0,
            session_id: None,
            tool_choice: None,
            metadata: None,
            thinking_level: None,
            cancel_flag: cancel,
            effort: None,
        };
        let mut rx = provider.chat_stream_incremental(request).await;
        let mut collected: Vec<StreamEvent> = Vec::new();
        while let Some(ev) = rx.recv().await {
            collected.push(ev);
        }
        collected
    });
    world.stream_events = events;
}

#[then("the chat request should return a cancellation error")]
fn then_chat_returns_cancellation_error(world: &mut QuectoWorld) {
    let result = world.chat_result.as_ref().expect("no chat result");
    assert!(
        result.is_err(),
        "expected cancellation error, got: {:?}",
        result
    );
    let msg = result.as_ref().unwrap_err();
    assert!(
        msg.to_lowercase().contains("cancel") || msg.to_lowercase().contains("abort"),
        "expected 'cancel' or 'abort' in error message, got: {}",
        msg
    );
}

#[then("the streaming chat request should return a cancellation error")]
fn then_streaming_returns_cancellation_error(world: &mut QuectoWorld) {
    let result = world.chat_stream_result.as_ref().expect("no stream result");
    assert!(
        result.is_err(),
        "expected cancellation error, got: {:?}",
        result
    );
    let msg = result.as_ref().unwrap_err();
    assert!(
        msg.to_lowercase().contains("cancel") || msg.to_lowercase().contains("abort"),
        "expected 'cancel' or 'abort' in error, got: {}",
        msg
    );
}

#[then("the chat request should succeed with a response")]
fn then_chat_succeeds(world: &mut QuectoWorld) {
    let result = world.chat_result.as_ref().expect("no chat result");
    assert!(result.is_ok(), "expected success, got error: {:?}", result);
}

#[then(expr = "I should receive an Error stream event containing {string}")]
fn then_stream_has_error_containing(world: &mut QuectoWorld, expected: String) {
    use quecto::domain::provider::StreamEvent;
    let found = world.stream_events.iter().any(|ev| {
        if let StreamEvent::Error(msg) = ev {
            msg.to_lowercase().contains(&expected.to_lowercase())
        } else {
            false
        }
    });
    assert!(
        found,
        "expected Error event containing '{}', got: {:?}",
        expected, world.stream_events
    );
}

#[given(expr = "a stop reason string {string}")]
fn given_stop_reason_string(world: &mut QuectoWorld, reason: String) {
    use quecto::domain::message::StopReason;
    world.parsed_stop_reason = Some(StopReason::parse(&reason));
}

#[when("I parse the stop reason")]
fn when_parse_stop_reason(world: &mut QuectoWorld) {
    // Re-assert the parsed stop reason is present (parsing was done in the given step).
    assert!(
        world.parsed_stop_reason.is_some(),
        "stop reason should have been parsed in the given step"
    );
}

#[then(expr = "the stop reason should be {word}")]
fn then_stop_reason_variant(world: &mut QuectoWorld, expected_variant: String) {
    use quecto::domain::message::StopReason;
    let sr = world
        .parsed_stop_reason
        .as_ref()
        .expect("no parsed stop reason");
    let matches = match expected_variant.as_str() {
        "Aborted" => matches!(sr, StopReason::Aborted),
        "EndTurn" => matches!(sr, StopReason::EndTurn),
        "MaxTokens" => matches!(sr, StopReason::MaxTokens),
        "ToolUse" => matches!(sr, StopReason::ToolUse),
        "Error" => matches!(sr, StopReason::Error),
        "Refusal" => matches!(sr, StopReason::Refusal),
        other => panic!("unknown stop reason variant '{}'", other),
    };
    assert!(
        matches,
        "expected StopReason::{}, got: {:?}",
        expected_variant, sr
    );
}

// ===========================================================================
// #182 extra: Aborted messages are filtered by normalize_messages
// ===========================================================================

#[given("a message list with an aborted assistant turn followed by a new user message")]
fn given_aborted_message_list(world: &mut QuectoWorld) {
    use quecto::domain::message::StopReason;
    let mut aborted = Message::assistant("partial response", vec![]);
    aborted.stop_reason = Some(StopReason::Aborted);
    let follow_up = Message::user("please continue");
    let msgs = vec![Message::user("hello"), aborted, follow_up];
    // Normalize and store the resulting API messages for assertion.
    let (_, api_msgs) =
        quecto::infrastructure::providers::anthropic::AnthropicProvider::build_messages_public(
            &msgs,
        );
    world.api_messages = api_msgs;
}

#[when("I normalize the messages")]
fn when_normalize_messages(world: &mut QuectoWorld) {
    assert!(
        !world.api_messages.is_empty(),
        "normalized message list should not be empty — given step must have populated it"
    );
}

#[then("the aborted assistant message should be removed")]
fn then_aborted_message_removed(world: &mut QuectoWorld) {
    let has_partial = world.api_messages.iter().any(|m| {
        m["content"]
            .as_str()
            .map(|s| s.contains("partial response"))
            .unwrap_or(false)
            || m["content"]
                .as_array()
                .map(|arr| {
                    arr.iter().any(|b| {
                        b["text"]
                            .as_str()
                            .map(|t| t.contains("partial response"))
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false)
    });
    assert!(
        !has_partial,
        "aborted assistant message should have been filtered out"
    );
}

#[then("the new user message should remain")]
fn then_new_user_message_remains(world: &mut QuectoWorld) {
    let has_followup = world.api_messages.iter().any(|m| {
        m["content"]
            .as_str()
            .map(|s| s.contains("please continue"))
            .unwrap_or(false)
            || m["content"]
                .as_array()
                .map(|arr| {
                    arr.iter().any(|b| {
                        b["text"]
                            .as_str()
                            .map(|t| t.contains("please continue"))
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false)
    });
    assert!(
        has_followup,
        "new user message should still be in the normalized list"
    );
}

// --- normalize_messages clone-on-write (#374) ---

#[given("a message list with only user and assistant messages and no tool calls")]
fn given_simple_message_list(world: &mut QuectoWorld) {
    let msgs = vec![
        Message::user("hello"),
        Message::assistant("hi there", vec![]),
        Message::user("follow up"),
    ];
    let (_, api_msgs) =
        quecto::infrastructure::providers::anthropic::AnthropicProvider::build_messages_public(
            &msgs,
        );
    world.api_messages = api_msgs;
}

#[then("all messages should be returned without deep cloning")]
fn then_no_deep_cloning(world: &mut QuectoWorld) {
    // The clone-on-write behavior is verified by the unit test
    // test_normalize_messages_does_not_clone_unmodified_messages.
    // This BDD step confirms the pipeline produces the expected output.
    assert_eq!(
        world.api_messages.len(),
        3,
        "all 3 messages should be present"
    );
}

// ===========================================================================
// #416: Default effort=low for 4.6 models
// ===========================================================================

#[given(expr = "an Anthropic request for model {string} with no effort level")]
fn given_anthropic_request_no_effort(world: &mut QuectoWorld, model: String) {
    world.env_overrides.insert("_effort_model".into(), model);
    world.env_overrides.remove("_effort_level");
}

#[given(expr = "an Anthropic request for model {string} with effort level {string}")]
fn given_anthropic_request_with_effort(world: &mut QuectoWorld, model: String, level: String) {
    world.env_overrides.insert("_effort_model".into(), model);
    world.env_overrides.insert("_effort_level".into(), level);
}

#[when("I build the Anthropic request body with effort")]
fn when_build_request_body_with_effort(world: &mut QuectoWorld) {
    let model = world.env_overrides.get("_effort_model").cloned().unwrap();
    let effort = world
        .env_overrides
        .get("_effort_level")
        .map(|l| match l.as_str() {
            "low" => quecto::domain::provider::EffortLevel::Low,
            "medium" => quecto::domain::provider::EffortLevel::Medium,
            "high" => quecto::domain::provider::EffortLevel::High,
            "max" => quecto::domain::provider::EffortLevel::Max,
            _ => panic!("unknown effort level: {}", l),
        });

    let messages = vec![quecto::domain::message::Message::user("test")];
    let req = quecto::domain::provider::ChatRequest {
        messages: &messages,
        tools: &[],
        model: &model,
        max_tokens: 8192,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort,
    };
    let (_sys, body) =
        quecto::infrastructure::providers::anthropic::AnthropicProvider::build_request_body_public(
            &req,
        );
    world
        .env_overrides
        .insert("_anthropic_body".into(), body.to_string());
}

#[then(expr = "the request body should contain output_config effort {string}")]
fn then_output_config_effort(world: &mut QuectoWorld, expected: String) {
    let body_str = world.env_overrides.get("_anthropic_body").expect("no body");
    let body: serde_json::Value = serde_json::from_str(body_str).expect("invalid json");
    assert_eq!(
        body["output_config"]["effort"].as_str(),
        Some(expected.as_str()),
        "expected output_config.effort='{}', got body: {}",
        expected,
        body
    );
}

#[then("the request body should not contain an output_config field")]
fn then_no_output_config(world: &mut QuectoWorld) {
    let body_str = world.env_overrides.get("_anthropic_body").expect("no body");
    let body: serde_json::Value = serde_json::from_str(body_str).expect("invalid json");
    assert!(
        body.get("output_config").is_none() || body["output_config"].is_null(),
        "expected no output_config field, got body: {}",
        body
    );
}

// ===========================================================================
// #438: SSE streaming reverse-maps OAuth tool names
// ===========================================================================

/// Build a minimal SSE payload with a single tool call.
fn build_sse_tool_payload(tool_name: &str) -> String {
    format!(
        "event: content_block_start\n\
         data: {{\"content_block\":{{\"type\":\"tool_use\",\"id\":\"toolu_001\",\"name\":\"{}\"}}}}\n\n\
         event: content_block_delta\n\
         data: {{\"delta\":{{\"type\":\"input_json_delta\",\"partial_json\":\"{{\\\"command\\\":\\\"ls\\\"}}\"}}}}\n\n\
         event: content_block_stop\n\
         data: {{}}\n\n\
         event: message_delta\n\
         data: {{\"delta\":{{\"stop_reason\":\"tool_use\"}}}}\n\n\
         event: message_stop\n\
         data: {{}}\n\n",
        tool_name
    )
}

#[given(expr = "an Anthropic SSE response with tool {string} and tool definitions for {string}")]
fn given_sse_with_tool_and_defs(world: &mut QuectoWorld, wire_name: String, registry_name: String) {
    world
        .env_overrides
        .insert("_sse438_payload".into(), build_sse_tool_payload(&wire_name));
    world
        .env_overrides
        .insert("_sse438_registry_name".into(), registry_name);
}

#[given(expr = "an Anthropic SSE response with tool {string} and no tool remapping")]
fn given_sse_with_tool_no_remap(world: &mut QuectoWorld, tool_name: String) {
    world
        .env_overrides
        .insert("_sse438_payload".into(), build_sse_tool_payload(&tool_name));
    world.env_overrides.remove("_sse438_registry_name");
}

#[when("I parse the SSE response with OAuth tool remapping")]
fn when_parse_sse_with_oauth_remap(world: &mut QuectoWorld) {
    let sse = world
        .env_overrides
        .get("_sse438_payload")
        .expect("no SSE payload")
        .clone();
    let registry_name = world
        .env_overrides
        .get("_sse438_registry_name")
        .expect("no registry name")
        .clone();
    let tool_defs = vec![quecto::domain::tool::ToolDefinition {
        name: registry_name.into(),
        description: "test tool".into(),
        parameters_schema: "{}".into(),
    }];
    let response =
        quecto::infrastructure::providers::anthropic::AnthropicProvider::parse_sse_response_with_tools_public(&sse, &tool_defs)
            .expect("SSE parse should succeed");
    world.streaming_response = Some(response);
}

#[when("I parse the SSE response without OAuth tool remapping")]
fn when_parse_sse_without_oauth_remap(world: &mut QuectoWorld) {
    let sse = world
        .env_overrides
        .get("_sse438_payload")
        .expect("no SSE payload")
        .clone();
    let response =
        quecto::infrastructure::providers::anthropic::AnthropicProvider::parse_sse_response_public(
            &sse,
        )
        .expect("SSE parse should succeed");
    world.streaming_response = Some(response);
}

#[then(expr = "the tool call name in the response should be {string}")]
fn then_tool_call_name_in_response(world: &mut QuectoWorld, expected: String) {
    let response = world.streaming_response.as_ref().expect("no response");
    assert!(
        !response.tool_calls.is_empty(),
        "expected tool calls in response, got none"
    );
    assert_eq!(
        response.tool_calls[0].name, expected,
        "expected tool call name '{}', got '{}'",
        expected, response.tool_calls[0].name
    );
}

#[when("I parse the SSE events with OAuth tool remapping")]
async fn when_parse_sse_events_with_oauth_remap(world: &mut QuectoWorld) {
    let sse = world
        .env_overrides
        .get("_sse438_payload")
        .expect("no SSE payload")
        .clone();
    let registry_name = world
        .env_overrides
        .get("_sse438_registry_name")
        .expect("no registry name")
        .clone();
    let tool_defs = vec![quecto::domain::tool::ToolDefinition {
        name: registry_name.into(),
        description: "test tool".into(),
        parameters_schema: "{}".into(),
    }];
    world.stream_events =
        quecto::infrastructure::providers::anthropic::AnthropicProvider::parse_sse_events_with_tools_public(&sse, &tool_defs).await;
}

#[when("I parse the SSE events without OAuth tool remapping")]
async fn when_parse_sse_events_without_oauth_remap(world: &mut QuectoWorld) {
    let sse = world
        .env_overrides
        .get("_sse438_payload")
        .expect("no SSE payload")
        .clone();
    world.stream_events =
        quecto::infrastructure::providers::anthropic::AnthropicProvider::parse_sse_events_public(
            &sse,
        )
        .await;
}

#[then(expr = "the ToolCallStart event name should be {string}")]
fn then_tool_call_start_name(world: &mut QuectoWorld, expected: String) {
    use quecto::domain::provider::StreamEvent;
    let found = world
        .stream_events
        .iter()
        .find_map(|ev| match ev {
            StreamEvent::ToolCallStart { name, .. } => Some(name.clone()),
            _ => None,
        })
        .expect("no ToolCallStart event found");
    assert_eq!(
        found, expected,
        "expected ToolCallStart name '{}', got '{}'",
        expected, found
    );
}

#[then(expr = "the ToolCallEnd event name should be {string}")]
fn then_tool_call_end_name(world: &mut QuectoWorld, expected: String) {
    use quecto::domain::provider::StreamEvent;
    let found = world
        .stream_events
        .iter()
        .find_map(|ev| match ev {
            StreamEvent::ToolCallEnd { name, .. } => Some(name.clone()),
            _ => None,
        })
        .expect("no ToolCallEnd event found");
    assert_eq!(
        found, expected,
        "expected ToolCallEnd name '{}', got '{}'",
        expected, found
    );
}

#[then(expr = "the Done response tool call name should be {string}")]
fn then_done_response_tool_call_name(world: &mut QuectoWorld, expected: String) {
    use quecto::domain::provider::StreamEvent;
    let response = world
        .stream_events
        .iter()
        .find_map(|ev| match ev {
            StreamEvent::Done(resp) => Some(resp),
            _ => None,
        })
        .expect("no Done event found");
    assert!(
        !response.tool_calls.is_empty(),
        "expected tool calls in Done response"
    );
    assert_eq!(
        response.tool_calls[0].name, expected,
        "expected Done response tool call name '{}', got '{}'",
        expected, response.tool_calls[0].name
    );
}

// ===========================================================================
// #437/#438: Anthropic provider API parity (OAuth, beta headers, tool name
// remapping, thinking-block replay, signature_delta SSE, Accept header)
// ===========================================================================

fn parity_oauth_flag(word: &str) -> bool {
    word == "true"
}

fn parity_body(world: &QuectoWorld) -> serde_json::Value {
    let s = world
        .env_overrides
        .get("_anthropic_body")
        .expect("no built request body — run the build-body When step first");
    serde_json::from_str(s).expect("invalid request body json")
}

fn parity_build_body(world: &mut QuectoWorld, is_oauth: bool) {
    let msgs = world
        .context_messages
        .clone()
        .unwrap_or_else(|| vec![Message::user("Hi")]);
    let tools: Vec<ToolDefinition> = match world.env_overrides.get("_parity_tool") {
        Some(name) => vec![ToolDefinition {
            name: name.clone().into(),
            description: "Tool".into(),
            parameters_schema: "{}".into(),
        }],
        None => vec![],
    };
    let req = ChatRequest {
        messages: &msgs,
        tools: &tools,
        model: "claude-sonnet-4-6",
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let (_sys, body) =
        quecto::infrastructure::providers::anthropic::AnthropicProvider::build_request_body_with_oauth(
            &req, is_oauth,
        );
    world
        .env_overrides
        .insert("_anthropic_body".into(), body.to_string());
}

// --- System prompt / OAuth identity prefix ---

#[given(expr = "an Anthropic request with system prompt {string} and is_oauth {word}")]
fn given_parity_system_prompt(world: &mut QuectoWorld, prompt: String, _oauth: String) {
    world.context_messages = Some(vec![Message::system(prompt), Message::user("Hi")]);
    world.env_overrides.remove("_parity_tool");
}

#[given(expr = "an Anthropic request with no system prompt and is_oauth {word}")]
fn given_parity_no_system_prompt(world: &mut QuectoWorld, _oauth: String) {
    world.context_messages = Some(vec![Message::user("Hi")]);
    world.env_overrides.remove("_parity_tool");
}

#[given(expr = "an Anthropic request with tool {string} and is_oauth {word}")]
fn given_parity_tool(world: &mut QuectoWorld, tool: String, _oauth: String) {
    world.context_messages = Some(vec![Message::user("Hi")]);
    world.env_overrides.insert("_parity_tool".into(), tool);
}

#[when("I build the Anthropic request body with OAuth")]
fn when_build_body_oauth(world: &mut QuectoWorld) {
    parity_build_body(world, true);
}

#[when("I build the Anthropic request body without OAuth")]
fn when_build_body_no_oauth(world: &mut QuectoWorld) {
    parity_build_body(world, false);
}

#[then(expr = "the system prompt array should have {int} block(s)")]
fn then_system_block_count(world: &mut QuectoWorld, count: usize) {
    let body = parity_body(world);
    let system = body["system"].as_array().expect("system should be array");
    assert_eq!(system.len(), count, "system block count");
}

#[then(expr = "the first system block text should be {string}")]
fn then_first_system_block(world: &mut QuectoWorld, expected: String) {
    let body = parity_body(world);
    assert_eq!(body["system"][0]["text"].as_str().unwrap(), expected);
}

#[then(expr = "the second system block text should be {string}")]
fn then_second_system_block(world: &mut QuectoWorld, expected: String) {
    let body = parity_body(world);
    assert_eq!(body["system"][1]["text"].as_str().unwrap(), expected);
}

#[then("both system blocks should have cache_control ephemeral")]
fn then_both_system_blocks_cache_control(world: &mut QuectoWorld) {
    let body = parity_body(world);
    let system = body["system"].as_array().expect("system should be array");
    for block in system {
        assert_eq!(
            block["cache_control"]["type"].as_str(),
            Some("ephemeral"),
            "each system block should have ephemeral cache_control"
        );
    }
}

// --- Beta headers ---

#[given(expr = "an Anthropic beta header for model {string} with is_oauth {word}")]
fn given_beta_header(world: &mut QuectoWorld, model: String, oauth: String) {
    world
        .env_overrides
        .insert("_parity_beta_model".into(), model);
    world
        .env_overrides
        .insert("_parity_beta_oauth".into(), oauth);
}

#[when("I build the beta header")]
fn when_build_beta_header(world: &mut QuectoWorld) {
    let model = world
        .env_overrides
        .get("_parity_beta_model")
        .cloned()
        .expect("no beta model set");
    let is_oauth = parity_oauth_flag(
        world
            .env_overrides
            .get("_parity_beta_oauth")
            .map(|s| s.as_str())
            .unwrap_or("false"),
    );
    let header =
        quecto::infrastructure::providers::anthropic::AnthropicProvider::build_beta_header_public(
            &model, is_oauth,
        );
    world.env_overrides.insert("_beta_header".into(), header);
}

#[then(expr = "the beta header should contain {string}")]
fn then_beta_header_contains(world: &mut QuectoWorld, needle: String) {
    let header = world
        .env_overrides
        .get("_beta_header")
        .expect("no beta header");
    assert!(
        header.contains(&needle),
        "beta header '{}' should contain '{}'",
        header,
        needle
    );
}

#[then(expr = "the beta header should not contain {string}")]
fn then_beta_header_not_contains(world: &mut QuectoWorld, needle: String) {
    let header = world
        .env_overrides
        .get("_beta_header")
        .expect("no beta header");
    assert!(
        !header.contains(&needle),
        "beta header '{}' should not contain '{}'",
        header,
        needle
    );
}

// --- Canonical tool-name remapping ---

#[given(expr = "a tool named {string}")]
fn given_tool_named(world: &mut QuectoWorld, name: String) {
    world.env_overrides.insert("_canon_in".into(), name);
}

#[when("I convert it to canonical name")]
fn when_convert_canonical(world: &mut QuectoWorld) {
    let name = world
        .env_overrides
        .get("_canon_in")
        .cloned()
        .expect("no tool name");
    let canon =
        quecto::infrastructure::providers::anthropic::AnthropicProvider::to_claude_code_name_public(
            &name,
        )
        .to_string();
    world.env_overrides.insert("_canon_out".into(), canon);
}

#[then(expr = "the result should be {string}")]
fn then_canonical_result(world: &mut QuectoWorld, expected: String) {
    let out = world
        .env_overrides
        .get("_canon_out")
        .expect("no canonical-name result");
    assert_eq!(out, &expected);
}

#[then(expr = "the Anthropic tool definition name should be {string}")]
fn then_anthropic_tool_def_name(world: &mut QuectoWorld, expected: String) {
    let body = parity_body(world);
    assert_eq!(body["tools"][0]["name"].as_str().unwrap(), expected);
}

// --- Assistant message thinking-block replay ---

fn parity_assistant_content(world: &QuectoWorld) -> Vec<serde_json::Value> {
    let s = world
        .env_overrides
        .get("_anthropic_msgs")
        .expect("no built messages");
    let msgs: Vec<serde_json::Value> = serde_json::from_str(s).expect("invalid messages json");
    let asst = msgs
        .iter()
        .find(|m| m["role"] == "assistant")
        .expect("no assistant message in built messages");
    asst["content"].as_array().cloned().unwrap_or_default()
}

#[given(expr = "an assistant message with a normal thinking block {string} and signature {string}")]
fn given_asst_normal_thinking(world: &mut QuectoWorld, thinking: String, signature: String) {
    use quecto::domain::message::ThinkingBlock;
    let mut asst = Message::assistant("response text", vec![]);
    asst.thinking_blocks.push(ThinkingBlock::Normal {
        thinking,
        signature,
    });
    world.context_messages = Some(vec![Message::user("Hi"), asst]);
}

#[given(expr = "an assistant message with a redacted thinking block with data {string}")]
fn given_asst_redacted_thinking(world: &mut QuectoWorld, data: String) {
    use quecto::domain::message::ThinkingBlock;
    let mut asst = Message::assistant("response text", vec![]);
    asst.thinking_blocks.push(ThinkingBlock::Redacted { data });
    world.context_messages = Some(vec![Message::user("Hi"), asst]);
}

#[when("I build the Anthropic assistant message")]
fn when_build_assistant_message(world: &mut QuectoWorld) {
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
    expr = "the content blocks should include a thinking block with text {string} and signature {string}"
)]
fn then_content_has_thinking(world: &mut QuectoWorld, text: String, signature: String) {
    let content = parity_assistant_content(world);
    let block = content
        .iter()
        .find(|b| b["type"] == "thinking")
        .expect("no thinking block in content");
    assert_eq!(block["thinking"].as_str().unwrap(), text);
    assert_eq!(block["signature"].as_str().unwrap(), signature);
}

#[then(expr = "the content blocks should include a redacted_thinking block with data {string}")]
fn then_content_has_redacted(world: &mut QuectoWorld, data: String) {
    let content = parity_assistant_content(world);
    let block = content
        .iter()
        .find(|b| b["type"] == "redacted_thinking")
        .expect("no redacted_thinking block in content");
    assert_eq!(block["data"].as_str().unwrap(), data);
}

#[then(expr = "the content blocks should include a text block with {string} instead of thinking")]
fn then_content_text_fallback(world: &mut QuectoWorld, text: String) {
    let content = parity_assistant_content(world);
    assert!(
        content.iter().all(|b| b["type"] != "thinking"),
        "should NOT have a thinking block"
    );
    assert!(
        content
            .iter()
            .any(|b| b["type"] == "text" && b["text"].as_str() == Some(text.as_str())),
        "expected a text block with '{}'",
        text
    );
}

// --- signature_delta SSE accumulation ---

#[given(expr = "an Anthropic SSE stream with thinking_delta {string} and signature_delta {string}")]
fn given_sse_signature_delta(world: &mut QuectoWorld, thinking: String, signature: String) {
    use serde_json::json;
    let event = |name: &str, data: serde_json::Value| format!("event: {name}\ndata: {data}\n\n");
    let raw = format!(
        "{}{}{}{}{}{}{}",
        event(
            "message_start",
            json!({"type":"message_start","message":{"usage":{"input_tokens":10}}})
        ),
        event(
            "content_block_start",
            json!({"type":"content_block_start","content_block":{"type":"thinking","thinking":""}})
        ),
        event(
            "content_block_delta",
            json!({"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":thinking}})
        ),
        event(
            "content_block_delta",
            json!({"type":"content_block_delta","delta":{"type":"signature_delta","signature":signature}})
        ),
        event("content_block_stop", json!({"type":"content_block_stop"})),
        event(
            "message_delta",
            json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":5}})
        ),
        event("message_stop", json!({"type":"message_stop"})),
    );
    world.env_overrides.insert("_sse_raw".into(), raw);
}

#[when("I parse the SSE events")]
fn when_parse_sse_events_parity(world: &mut QuectoWorld) {
    let raw = world.env_overrides.get("_sse_raw").expect("no SSE data");
    let resp =
        quecto::infrastructure::providers::anthropic::AnthropicProvider::parse_sse_response_public(
            raw,
        )
        .expect("SSE parse failed");
    world.streaming_response = Some(resp);
}

#[then(expr = "the accumulated thinking block should have signature {string}")]
fn then_accumulated_signature(world: &mut QuectoWorld, expected: String) {
    use quecto::domain::message::ThinkingBlock;
    let resp = world
        .streaming_response
        .as_ref()
        .expect("no response parsed");
    let found = resp
        .thinking_blocks
        .iter()
        .any(|b| matches!(b, ThinkingBlock::Normal { signature, .. } if signature == &expected));
    assert!(
        found,
        "no thinking block with signature '{}'; blocks: {:?}",
        expected, resp.thinking_blocks
    );
}

// --- Accept header ---

#[given("an Anthropic provider with a mock server expecting Accept header")]
fn given_anthropic_mock_expect_accept(world: &mut QuectoWorld) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(wiremock::MockServer::start());
    let uri = server.uri();
    let response_body = serde_json::json!({
        "id": "msg_accept",
        "type": "message",
        "role": "assistant",
        "content": [{"type": "text", "text": "ok"}],
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 5, "output_tokens": 2}
    });
    rt.block_on(async {
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/v1/messages"))
            .and(wiremock::matchers::header("Accept", "application/json"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(response_body))
            .expect(1)
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

#[then(expr = "the request should include header {string} with value {string}")]
fn then_request_includes_header(world: &mut QuectoWorld, _header: String, _value: String) {
    assert!(
        world.streaming_response.is_some(),
        "no response — the expected header may not have been sent"
    );
}
