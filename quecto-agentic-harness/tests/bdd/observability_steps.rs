use super::*;

use quecto::infrastructure::logging::redact_api_keys;

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
        thinking_blocks: vec![],
    });
    // Second response: text
    mock_llm.push_response(LlmResponse {
        content: Some("Done".to_string()),
        tool_calls: vec![],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
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
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
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
        *output
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
        *output
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
        *output
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
        *output
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
