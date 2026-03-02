use super::*;

use quecto::infrastructure::health::server::{HealthServer, StaticReadiness};
use quecto::infrastructure::logging::redact_api_keys;

// ===========================================================================
// Health Server Steps
// ===========================================================================

#[given("a health server started on a random port")]
fn given_health_server(world: &mut QuectoWorld) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let readiness = Arc::new(StaticReadiness::new(true));
    let (addr, readiness_ref) = rt.block_on(async {
        let server = HealthServer::bind("127.0.0.1:0", readiness.clone())
            .await
            .expect("health server should bind");
        let addr = server.local_addr().expect("should have addr");
        tokio::spawn(async move { server.run().await });
        (addr, readiness)
    });
    world.health_server_addr = Some(addr.to_string());
    world.health_readiness = Some(readiness_ref);
    // Leak the runtime so the spawned server task stays alive.
    std::mem::forget(rt);
}

#[given("the readiness check reports providers available")]
fn given_readiness_available(world: &mut QuectoWorld) {
    if let Some(ref r) = world.health_readiness {
        r.set_ready(true);
    }
}

#[given("the readiness check reports no providers available")]
fn given_readiness_unavailable(world: &mut QuectoWorld) {
    if let Some(ref r) = world.health_readiness {
        r.set_ready(false);
    }
}

#[when(expr = "I request GET {string} from the health server")]
fn when_request_health(world: &mut QuectoWorld, path: String) {
    let addr = world
        .health_server_addr
        .as_ref()
        .expect("health server not started");
    let url = format!("http://{}{}", addr, path);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let resp = rt
        .block_on(async { reqwest::Client::new().get(&url).send().await })
        .expect("HTTP request should succeed");
    world.health_response_status = Some(resp.status().as_u16());
    let body = rt.block_on(resp.text()).expect("should read body");
    world.health_response_body = Some(body);
}

#[then(expr = "the HTTP response status should be {int}")]
fn then_http_status(world: &mut QuectoWorld, expected: u16) {
    let status = world.health_response_status.expect("no health response");
    assert_eq!(
        status, expected,
        "expected HTTP {}, got {}",
        expected, status
    );
}

#[then(expr = "the response body should be JSON containing {string} with value {string}")]
fn then_body_json_contains(world: &mut QuectoWorld, key: String, value: String) {
    let body = world
        .health_response_body
        .as_ref()
        .expect("no response body");
    let json: serde_json::Value =
        serde_json::from_str(body).expect("response should be valid JSON");
    let actual = &json[&key];
    // Handle both string and boolean values
    let matches = match actual {
        serde_json::Value::Bool(b) => b.to_string() == value,
        serde_json::Value::String(s) => s == &value,
        _ => actual.to_string().trim_matches('"') == value,
    };
    assert!(
        matches,
        "expected JSON key '{}' to have value '{}', got: {}",
        key, value, actual
    );
}

// ===========================================================================
// Structured Logging Steps
// ===========================================================================

#[given("an agent loop with a mock provider and mock tools")]
fn given_agent_with_mock_for_logging(world: &mut QuectoWorld) {
    // Set up a mock LLM that returns a tool call, then text
    let mock_llm = Arc::new(MockLlmProvider::new());
    // First response: tool call
    mock_llm.push_response(LlmResponse {
        content: None,
        tool_calls: vec![ToolCall {
            id: "call_test".to_string(),
            name: "bash".to_string(),
            arguments: r#"{"command":"echo hi"}"#.to_string(),
        }],
        usage: None,
        stop_reason: None,
    });
    // Second response: text
    mock_llm.push_response(LlmResponse {
        content: Some("Done".to_string()),
        tool_calls: vec![],
        usage: None,
        stop_reason: None,
    });
    world.mock_llm = Some(mock_llm);

    // Register a mock tool
    let tool = Arc::new(MockBddTool::new("bash", "hi"));
    let mut registry = quecto::infrastructure::tools::registry::ToolRegistryImpl::new();
    registry.register(tool);
    world.tool_registry = Some(registry);
}

#[given("a tracing subscriber capturing JSON log output")]
fn given_tracing_subscriber(world: &mut QuectoWorld) {
    // We capture log output via a buffer
    world.captured_log_output = Some(Arc::new(Mutex::new(String::new())));
}

#[when("the agent processes a message that triggers a tool call")]
fn when_agent_processes_tool_call(world: &mut QuectoWorld) {
    let mock_llm = world.mock_llm.take().expect("mock LLM not set");
    let registry = world.tool_registry.take().expect("registry not set");

    let agent = AgentLoopImpl::new(quecto::application::agent_loop::AgentLoopConfig {
        provider: mock_llm,
        tool_registry: Box::new(registry),
        model: "test-model".to_string(),
        max_tokens: 1024,
        temperature: 0.7,
        spill_store: None,
        session_key: String::new(),
        context_collapse_after_turns: u32::MAX,
        max_context_tokens: 190_000,
        progress_callback: None,
    });

    // Capture tracing output
    let buffer = world
        .captured_log_output
        .clone()
        .unwrap_or_else(|| Arc::new(Mutex::new(String::new())));
    let buf_clone = buffer.clone();

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        // Set up a JSON tracing subscriber that writes to our buffer
        let writer = LogWriter(buf_clone);
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_writer(move || writer.clone())
            .with_target(true)
            .finish();

        let _guard = tracing::subscriber::set_default(subscriber);

        let mut messages = vec![Message::user("run a command")];
        let result = agent.process(&mut messages).await;
        assert!(result.is_ok(), "agent should succeed: {:?}", result);
    });

    world.captured_log_output = Some(buffer);
}

#[when(expr = "the message {string} is logged at info level")]
fn when_message_logged(world: &mut QuectoWorld, message: String) {
    let buffer = world
        .captured_log_output
        .clone()
        .unwrap_or_else(|| Arc::new(Mutex::new(String::new())));
    let buf_clone = buffer.clone();

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let writer = LogWriter(buf_clone);
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_writer(move || writer.clone())
            .with_target(true)
            .finish();

        let _guard = tracing::subscriber::set_default(subscriber);
        // Redact the message before logging
        let redacted = redact_api_keys(&message);
        tracing::info!("{}", redacted);
    });

    world.captured_log_output = Some(buffer);
}

#[then(expr = "the captured log output should include span {string}")]
fn then_log_includes_span(world: &mut QuectoWorld, span_name: String) {
    let buffer = world
        .captured_log_output
        .as_ref()
        .expect("no captured output");
    let output = buffer.lock().unwrap();
    assert!(
        output.contains(&span_name),
        "expected log to include span '{}', got:\n{}",
        span_name,
        &*output
    );
}

#[then(expr = "the captured log output should include field {string}")]
fn then_log_includes_field(world: &mut QuectoWorld, field_name: String) {
    let buffer = world
        .captured_log_output
        .as_ref()
        .expect("no captured output");
    let output = buffer.lock().unwrap();
    assert!(
        output.contains(&field_name),
        "expected log to include field '{}', got:\n{}",
        field_name,
        &*output
    );
}

#[then(expr = "the captured log output should not contain {string}")]
fn then_log_not_contain(world: &mut QuectoWorld, unexpected: String) {
    let buffer = world
        .captured_log_output
        .as_ref()
        .expect("no captured output");
    let output = buffer.lock().unwrap();
    assert!(
        !output.contains(&unexpected),
        "expected log NOT to contain '{}', but got:\n{}",
        unexpected,
        &*output
    );
}

#[then("the captured log output should contain a redacted placeholder")]
fn then_log_contains_redacted(world: &mut QuectoWorld) {
    let buffer = world
        .captured_log_output
        .as_ref()
        .expect("no captured output");
    let output = buffer.lock().unwrap();
    assert!(
        output.contains("sk-***") || output.contains("***"),
        "expected log to contain redacted placeholder, got:\n{}",
        &*output
    );
}

// ===========================================================================
// Tracing writer helper
// ===========================================================================

#[derive(Clone)]
struct LogWriter(Arc<Mutex<String>>);

impl std::io::Write for LogWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let s = String::from_utf8_lossy(buf);
        self.0.lock().unwrap().push_str(&s);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl tracing_subscriber::fmt::MakeWriter<'_> for LogWriter {
    type Writer = Self;

    fn make_writer(&self) -> Self::Writer {
        self.clone()
    }
}
