use super::*;
use crate::domain::message::{LlmResponse, Role, ToolCall, UsageInfo};
use crate::domain::tool::{Tool, ToolDefinition, ToolRegistry, ToolResult};
use std::sync::{Arc, Mutex};

// -----------------------------------------------------------------------
// Mock LLM Provider for unit tests
// -----------------------------------------------------------------------

#[derive(Debug)]
struct MockProvider {
    responses: Mutex<Vec<LlmResponse>>,
    /// Captured tool definitions from the last chat() call.
    last_tool_defs: Mutex<Vec<ToolDefinition>>,
}

impl MockProvider {
    fn new(responses: Vec<LlmResponse>) -> Self {
        Self {
            responses: Mutex::new(responses),
            last_tool_defs: Mutex::new(vec![]),
        }
    }

    fn last_tool_defs(&self) -> Vec<ToolDefinition> {
        self.last_tool_defs.lock().unwrap().clone()
    }
}

impl LlmProvider for MockProvider {
    fn name(&self) -> &str {
        "mock"
    }

    fn chat(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<LlmResponse, DomainError>> + Send + '_>>
    {
        // Capture tool defs
        *self.last_tool_defs.lock().unwrap() = request.tools.to_vec();

        let response = {
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                return Box::pin(async {
                    Ok(LlmResponse {
                        content: Some("(no more responses)".to_string()),
                        tool_calls: vec![],
                        usage: None,
                    })
                });
            }
            responses.remove(0)
        };

        Box::pin(async move { Ok(response) })
    }
}

// -----------------------------------------------------------------------
// Mock Tool Registry for unit tests (application layer only)
// -----------------------------------------------------------------------

#[derive(Default)]
struct MockRegistry {
    tools: Vec<Arc<dyn Tool>>,
}

impl MockRegistry {
    fn new() -> Self {
        Self { tools: Vec::new() }
    }

    fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.push(tool);
    }
}

impl ToolRegistry for MockRegistry {
    fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.iter().map(|tool| tool.definition()).collect()
    }

    fn execute(
        &self,
        name: &str,
        arguments: &str,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<ToolResult, DomainError>> + Send + '_>>
    {
        if let Some(tool) = self
            .tools
            .iter()
            .find(|tool| tool.definition().name == name)
        {
            return tool.execute(arguments);
        }
        let err = DomainError::Tool(format!("unknown tool: {}", name));
        Box::pin(async move { Err(err) })
    }
}

// -----------------------------------------------------------------------
// Mock Tool for unit tests
// -----------------------------------------------------------------------

struct MockTool {
    def: ToolDefinition,
    response: Mutex<String>,
}

impl MockTool {
    fn new(name: &str, response: &str) -> Self {
        Self {
            def: ToolDefinition {
                name: name.to_string(),
                description: format!("Mock {} tool", name),
                parameters_schema: r#"{"type":"object"}"#.to_string(),
            },
            response: Mutex::new(response.to_string()),
        }
    }
}

impl std::fmt::Debug for MockTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockTool")
            .field("name", &self.def.name)
            .finish()
    }
}

impl crate::domain::tool::Tool for MockTool {
    fn definition(&self) -> ToolDefinition {
        self.def.clone()
    }

    fn execute(
        &self,
        _arguments: &str,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<ToolResult, DomainError>> + Send + '_>>
    {
        let content = self.response.lock().unwrap().clone();
        Box::pin(async move {
            Ok(ToolResult {
                content,
                is_error: false,
                image_blocks: vec![],
            })
        })
    }
}

// -----------------------------------------------------------------------
// Helper to build an AgentLoopImpl with mock components
// -----------------------------------------------------------------------

fn make_agent(
    responses: Vec<LlmResponse>,
    tools: Vec<(&str, &str)>,
) -> (AgentLoopImpl, Arc<MockProvider>) {
    let provider = Arc::new(MockProvider::new(responses));
    let mut registry = MockRegistry::new();
    for (name, response) in tools {
        registry.register(Arc::new(MockTool::new(name, response)));
    }
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: provider.clone(),
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
    (agent, provider)
}

fn text_response(content: &str) -> LlmResponse {
    LlmResponse {
        content: Some(content.to_string()),
        tool_calls: vec![],
        usage: Some(UsageInfo {
            prompt_tokens: 10,
            completion_tokens: 20,
        }),
    }
}

fn tool_call_response(name: &str, args: &str) -> LlmResponse {
    LlmResponse {
        content: None,
        tool_calls: vec![ToolCall {
            id: format!("call_{}", name),
            name: name.to_string(),
            arguments: args.to_string(),
        }],
        usage: None,
    }
}

// -----------------------------------------------------------------------
// Unit tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_simple_text_response() {
    let (agent, _) = make_agent(vec![text_response("Hello, world!")], vec![]);
    let mut messages = vec![Message::user("Hi")];
    let result = agent.run_loop(&mut messages).await.unwrap();
    assert_eq!(result.response, "Hello, world!");
    assert_eq!(result.tool_iterations, 0);
    assert!(!result.iteration_limit_reached);
}

#[tokio::test]
async fn test_single_tool_call() {
    let (agent, _) = make_agent(
        vec![
            tool_call_response("read", r#"{"path":"notes.txt"}"#),
            text_response("Your notes say: Buy groceries"),
        ],
        vec![("read", "Buy groceries")],
    );
    let mut messages = vec![Message::user("What are my notes?")];
    let result = agent.run_loop(&mut messages).await.unwrap();
    assert_eq!(result.response, "Your notes say: Buy groceries");
    assert_eq!(result.tool_iterations, 1);
}

#[tokio::test]
async fn test_multiple_tool_calls_in_sequence() {
    let (agent, _) = make_agent(
        vec![
            tool_call_response("read", r#"{"path":"a.txt"}"#),
            tool_call_response("write", r#"{"path":"b.txt","content":"data"}"#),
            text_response("Done copying"),
        ],
        vec![("read", "file content"), ("write", "ok")],
    );
    let mut messages = vec![Message::user("Copy files")];
    let result = agent.run_loop(&mut messages).await.unwrap();
    assert_eq!(result.response, "Done copying");
    assert_eq!(result.tool_iterations, 2);
}

#[tokio::test]
async fn test_iteration_limit() {
    // LLM always returns tool calls — should stop at limit
    let responses: Vec<LlmResponse> = (0..10)
        .map(|i| tool_call_response("bash", &format!(r#"{{"cmd":"echo {}"}}"#, i)))
        .collect();
    let (agent, _) = make_agent(responses, vec![("bash", "output")]);
    let agent = agent.with_max_tool_iterations(3);

    let mut messages = vec![Message::user("Loop forever")];
    let result = agent.run_loop(&mut messages).await.unwrap();
    assert!(result.iteration_limit_reached);
    assert_eq!(result.tool_iterations, 3);
    assert!(result.response.contains("limit"));
}

#[tokio::test]
async fn test_tool_definitions_sent_to_llm() {
    let (agent, provider) = make_agent(vec![text_response("ok")], vec![("bash", ""), ("read", "")]);
    let mut messages = vec![Message::user("test")];
    let _ = agent.run_loop(&mut messages).await.unwrap();
    let defs = provider.last_tool_defs();
    assert_eq!(defs.len(), 2);
    let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    assert!(names.contains(&"bash"));
    assert!(names.contains(&"read"));
}

#[tokio::test]
async fn test_agent_info() {
    let (agent, _) = make_agent(vec![], vec![("bash", ""), ("read", ""), ("write", "")]);
    let agent = agent.with_skill_count(2);
    let info = agent.info();
    assert_eq!(info.tool_count, 3);
    assert_eq!(info.skill_count, 2);
}

#[tokio::test]
async fn test_messages_appended_during_loop() {
    let (agent, _) = make_agent(
        vec![
            tool_call_response("read", r#"{"path":"x"}"#),
            text_response("final"),
        ],
        vec![("read", "content")],
    );
    let mut messages = vec![Message::user("read")];
    let _ = agent.run_loop(&mut messages).await.unwrap();
    // Should have: User, Assistant(tool_call), Tool(result), Assistant(final)
    assert_eq!(messages.len(), 4);
    assert_eq!(messages[1].role, Role::Assistant);
    assert_eq!(messages[2].role, Role::Tool);
    assert_eq!(messages[3].role, Role::Assistant);
    assert_eq!(messages[3].content, "final");
}

#[tokio::test]
async fn test_tool_error_is_sent_back() {
    // Use a tool that doesn't exist in registry — the loop should handle gracefully
    let responses = vec![
        LlmResponse {
            content: None,
            tool_calls: vec![ToolCall {
                id: "call_1".to_string(),
                name: "nonexistent_tool".to_string(),
                arguments: "{}".to_string(),
            }],
            usage: None,
        },
        text_response("I got an error"),
    ];
    let provider = Arc::new(MockProvider::new(responses));
    let registry = MockRegistry::new(); // empty
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
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
    let mut messages = vec![Message::user("use a tool")];
    let result = agent.run_loop(&mut messages).await.unwrap();
    assert_eq!(result.response, "I got an error");
    // The tool result message should contain the error
    assert!(messages[2].content.contains("Error"));
}

#[tokio::test]
async fn test_default_max_iterations() {
    let (agent, _) = make_agent(vec![], vec![]);
    assert_eq!(agent.max_tool_iterations, DEFAULT_MAX_TOOL_ITERATIONS);
}

// -----------------------------------------------------------------------
// Progress callback tests
// -----------------------------------------------------------------------

/// Helper: build an AgentLoopImpl with a progress callback that records events.
fn make_agent_with_callback(
    responses: Vec<LlmResponse>,
    tools: Vec<(&str, &str)>,
) -> (
    AgentLoopImpl,
    Arc<MockProvider>,
    Arc<Mutex<Vec<crate::domain::agent::AgentProgressEvent>>>,
) {
    let provider = Arc::new(MockProvider::new(responses));
    let mut registry = MockRegistry::new();
    for (name, response) in tools {
        registry.register(Arc::new(MockTool::new(name, response)));
    }
    let events: Arc<Mutex<Vec<crate::domain::agent::AgentProgressEvent>>> =
        Arc::new(Mutex::new(Vec::new()));
    let events_clone = events.clone();
    let callback: crate::domain::agent::ProgressCallback = Arc::new(move |ev| {
        events_clone.lock().unwrap().push(ev);
    });
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: provider.clone(),
        tool_registry: Box::new(registry),
        model: "test-model".to_string(),
        max_tokens: 1024,
        temperature: 0.7,
        spill_store: None,
        session_key: String::new(),
        context_collapse_after_turns: u32::MAX,
        max_context_tokens: 190_000,
        progress_callback: Some(callback),
    });
    (agent, provider, events)
}

#[tokio::test]
async fn test_progress_callback_thinking_fired_before_llm_call() {
    let (agent, _, events) = make_agent_with_callback(vec![text_response("hello")], vec![]);
    let mut messages = vec![Message::user("hi")];
    agent.run_loop(&mut messages).await.unwrap();

    let fired = events.lock().unwrap();
    let has_thinking = fired
        .iter()
        .any(|e| matches!(e, crate::domain::agent::AgentProgressEvent::Thinking { .. }));
    assert!(has_thinking, "expected Thinking event, got: {:?}", *fired);
}

#[tokio::test]
async fn test_progress_callback_done_fired_on_text_response() {
    let (agent, _, events) = make_agent_with_callback(vec![text_response("hello")], vec![]);
    let mut messages = vec![Message::user("hi")];
    agent.run_loop(&mut messages).await.unwrap();

    let fired = events.lock().unwrap();
    let has_done = fired
        .iter()
        .any(|e| matches!(e, crate::domain::agent::AgentProgressEvent::Done));
    assert!(has_done, "expected Done event, got: {:?}", *fired);
}

#[tokio::test]
async fn test_progress_callback_tool_started_fired_for_each_tool_call() {
    let (agent, _, events) = make_agent_with_callback(
        vec![
            tool_call_response("bash", r#"{"command":"echo hi"}"#),
            text_response("done"),
        ],
        vec![("bash", "hi")],
    );
    let mut messages = vec![Message::user("run echo")];
    agent.run_loop(&mut messages).await.unwrap();

    let fired = events.lock().unwrap();
    let tool_started_count = fired
        .iter()
        .filter(|e| matches!(e, crate::domain::agent::AgentProgressEvent::ToolStarted { name, .. } if name == "bash"))
        .count();
    assert_eq!(
        tool_started_count, 1,
        "expected 1 ToolStarted(bash) event, got: {:?}",
        *fired
    );
}

#[tokio::test]
async fn test_progress_callback_tool_finished_fired_after_tool_executes() {
    let (agent, _, events) = make_agent_with_callback(
        vec![
            tool_call_response("bash", r#"{"command":"echo hi"}"#),
            text_response("done"),
        ],
        vec![("bash", "hi")],
    );
    let mut messages = vec![Message::user("run echo")];
    agent.run_loop(&mut messages).await.unwrap();

    let fired = events.lock().unwrap();
    let tool_finished = fired
        .iter()
        .find(|e| matches!(e, crate::domain::agent::AgentProgressEvent::ToolFinished { name, .. } if name == "bash"));
    assert!(
        tool_finished.is_some(),
        "expected ToolFinished(bash) event, got: {:?}",
        *fired
    );
}

#[tokio::test]
async fn test_progress_callback_event_order_thinking_tool_started_tool_finished_done() {
    let (agent, _, events) = make_agent_with_callback(
        vec![
            tool_call_response("bash", r#"{"command":"echo hi"}"#),
            text_response("done"),
        ],
        vec![("bash", "hi")],
    );
    let mut messages = vec![Message::user("run echo")];
    agent.run_loop(&mut messages).await.unwrap();

    let fired = events.lock().unwrap();

    // Find positions of key event types
    let thinking_pos = fired
        .iter()
        .position(|e| matches!(e, crate::domain::agent::AgentProgressEvent::Thinking { .. }));
    let tool_started_pos = fired.iter().position(|e| {
        matches!(
            e,
            crate::domain::agent::AgentProgressEvent::ToolStarted { .. }
        )
    });
    let tool_finished_pos = fired.iter().position(|e| {
        matches!(
            e,
            crate::domain::agent::AgentProgressEvent::ToolFinished { .. }
        )
    });
    let done_pos = fired
        .iter()
        .rposition(|e| matches!(e, crate::domain::agent::AgentProgressEvent::Done));

    assert!(thinking_pos.is_some(), "expected Thinking event");
    assert!(tool_started_pos.is_some(), "expected ToolStarted event");
    assert!(tool_finished_pos.is_some(), "expected ToolFinished event");
    assert!(done_pos.is_some(), "expected Done event");

    let t = thinking_pos.unwrap();
    let ts = tool_started_pos.unwrap();
    let tf = tool_finished_pos.unwrap();
    let d = done_pos.unwrap();

    assert!(t < ts, "Thinking should fire before ToolStarted");
    assert!(ts < tf, "ToolStarted should fire before ToolFinished");
    assert!(tf < d, "ToolFinished should fire before Done");
}

#[tokio::test]
async fn test_progress_callback_tool_finished_captures_duration_and_error_flag() {
    let (agent, _, events) = make_agent_with_callback(
        vec![
            tool_call_response("bash", r#"{"command":"echo hi"}"#),
            text_response("done"),
        ],
        vec![("bash", "output")],
    );
    let mut messages = vec![Message::user("run it")];
    agent.run_loop(&mut messages).await.unwrap();

    let fired = events.lock().unwrap();
    if let Some(crate::domain::agent::AgentProgressEvent::ToolFinished {
        name,
        arguments,
        duration_ms,
        is_error,
    }) = fired.iter().find(|e| {
        matches!(
            e,
            crate::domain::agent::AgentProgressEvent::ToolFinished { .. }
        )
    }) {
        assert_eq!(name, "bash");
        assert!(
            arguments.contains("echo hi"),
            "expected ToolFinished arguments to include command, got: {arguments}"
        );
        // duration_ms may be 0 in test environments, but must not panic
        let _ = *duration_ms;
        assert!(!is_error, "mock tool should not be an error");
    } else {
        panic!("expected ToolFinished event, got: {:?}", *fired);
    }
}

#[tokio::test]
async fn test_progress_callback_multiple_tool_calls_all_reported() {
    let (agent, _, events) = make_agent_with_callback(
        vec![
            tool_call_response("read", r#"{"path":"a.txt"}"#),
            tool_call_response("write", r#"{"path":"b.txt","content":"x"}"#),
            text_response("done"),
        ],
        vec![("read", "content"), ("write", "ok")],
    );
    let mut messages = vec![Message::user("copy a to b")];
    agent.run_loop(&mut messages).await.unwrap();

    let fired = events.lock().unwrap();
    let started: Vec<&str> = fired
        .iter()
        .filter_map(|e| {
            if let crate::domain::agent::AgentProgressEvent::ToolStarted { name, .. } = e {
                Some(name.as_str())
            } else {
                None
            }
        })
        .collect();
    assert!(
        started.contains(&"read"),
        "expected ToolStarted(read), got: {:?}",
        started
    );
    assert!(
        started.contains(&"write"),
        "expected ToolStarted(write), got: {:?}",
        started
    );
}

#[tokio::test]
async fn test_progress_callback_none_does_not_panic() {
    // Verify that having no callback at all does not change behaviour
    let (agent, _) = make_agent(vec![text_response("ok")], vec![]);
    let mut messages = vec![Message::user("hi")];
    let result = agent.run_loop(&mut messages).await.unwrap();
    assert_eq!(result.response, "ok");
}

#[tokio::test]
async fn test_progress_callback_tool_started_includes_arguments() {
    let (agent, _, events) = make_agent_with_callback(
        vec![
            tool_call_response("bash", r#"{"command":"echo hello world"}"#),
            text_response("done"),
        ],
        vec![("bash", "hello world")],
    );
    let mut messages = vec![Message::user("run it")];
    agent.run_loop(&mut messages).await.unwrap();

    let fired = events.lock().unwrap();
    if let Some(crate::domain::agent::AgentProgressEvent::ToolStarted { name, arguments }) =
        fired.iter().find(|e| {
            matches!(
                e,
                crate::domain::agent::AgentProgressEvent::ToolStarted { .. }
            )
        })
    {
        assert_eq!(name, "bash");
        // arguments should be the raw JSON — not truncated at the domain level
        assert!(!arguments.is_empty(), "arguments should not be empty");
        assert!(
            arguments.contains("echo hello world"),
            "arguments should contain the command, got: {arguments}"
        );
    } else {
        panic!("expected ToolStarted event, got: {:?}", *fired);
    }
}
