use super::*;
use crate::domain::message::{LlmResponse, Role, ToolCall, UsageInfo};
use crate::domain::tool::{Tool, ToolDefinition, ToolRegistry, ToolResult};
use std::sync::{Arc, Mutex};

// -----------------------------------------------------------------------
// Mock LLM Provider for unit tests
// -----------------------------------------------------------------------

#[derive(Debug)]
pub(super) struct MockProvider {
    responses: Mutex<Vec<Result<LlmResponse, DomainError>>>,
    /// Captured tool definitions from the last chat() call.
    last_tool_defs: Mutex<Vec<ToolDefinition>>,
    /// Captured first system prompts from every chat() call.
    seen_system_prompts: Mutex<Vec<String>>,
    request_count: Mutex<usize>,
}

impl MockProvider {
    pub(super) fn new(responses: Vec<LlmResponse>) -> Self {
        Self {
            responses: Mutex::new(responses.into_iter().map(Ok).collect()),
            last_tool_defs: Mutex::new(vec![]),
            seen_system_prompts: Mutex::new(vec![]),
            request_count: Mutex::new(0),
        }
    }

    fn new_results(responses: Vec<Result<LlmResponse, DomainError>>) -> Self {
        Self {
            responses: Mutex::new(responses),
            last_tool_defs: Mutex::new(vec![]),
            seen_system_prompts: Mutex::new(vec![]),
            request_count: Mutex::new(0),
        }
    }

    fn request_count(&self) -> usize {
        *self.request_count.lock().unwrap()
    }

    fn last_tool_defs(&self) -> Vec<ToolDefinition> {
        self.last_tool_defs.lock().unwrap().clone()
    }

    fn seen_system_prompts(&self) -> Vec<String> {
        self.seen_system_prompts.lock().unwrap().clone()
    }
}

#[derive(Debug)]
struct MockStreamingProvider {
    responses: Mutex<Vec<Vec<crate::domain::provider::StreamEvent>>>,
    request_count: Mutex<usize>,
}

impl MockStreamingProvider {
    fn new(responses: Vec<Vec<crate::domain::provider::StreamEvent>>) -> Self {
        Self {
            responses: Mutex::new(responses),
            request_count: Mutex::new(0),
        }
    }

    fn request_count(&self) -> usize {
        *self.request_count.lock().unwrap()
    }
}

impl LlmProvider for MockStreamingProvider {
    fn name(&self) -> &str {
        "mock-streaming"
    }

    fn chat(
        &self,
        _request: ChatRequest<'_>,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<LlmResponse, DomainError>> + Send + '_>>
    {
        Box::pin(async { unreachable!("streaming test should use chat_stream_incremental") })
    }

    fn chat_stream_incremental(
        &self,
        _request: ChatRequest<'_>,
    ) -> Pin<
        Box<
            dyn std::future::Future<
                    Output = tokio::sync::mpsc::Receiver<crate::domain::provider::StreamEvent>,
                > + Send
                + '_,
        >,
    > {
        *self.request_count.lock().unwrap() += 1;
        let events = self.responses.lock().unwrap().remove(0);
        Box::pin(async move {
            let (tx, rx) = tokio::sync::mpsc::channel(8);
            tokio::spawn(async move {
                for event in events {
                    let _ = tx.send(event).await;
                }
            });
            rx
        })
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
        *self.request_count.lock().unwrap() += 1;
        *self.last_tool_defs.lock().unwrap() = request.tools.to_vec();
        let system_prompt = request
            .messages
            .iter()
            .find(|m| m.role == Role::System && !m.is_manifest)
            .map(|m| m.content.clone());
        if let Some(prompt) = system_prompt {
            self.seen_system_prompts.lock().unwrap().push(prompt);
        }

        let response = {
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                return Box::pin(async {
                    Ok(LlmResponse {
                        content: Some("(no more responses)".to_string()),
                        tool_calls: vec![],
                        usage: None,
                        stop_reason: None,
                        thinking_blocks: vec![],
                    })
                });
            }
            responses.remove(0)
        };

        Box::pin(async move { response })
    }
}

// -----------------------------------------------------------------------
// Mock Tool Registry for unit tests (application layer only)
// -----------------------------------------------------------------------

#[derive(Default)]
pub(super) struct MockRegistry {
    tools: Vec<Arc<dyn Tool>>,
    cached_definitions: Vec<ToolDefinition>,
}

impl MockRegistry {
    pub(super) fn new() -> Self {
        Self {
            tools: Vec::new(),
            cached_definitions: Vec::new(),
        }
    }

    pub(super) fn register(&mut self, tool: Arc<dyn Tool>) {
        let def = tool.definition();
        self.cached_definitions.push(def);
        self.tools.push(tool);
    }
}

impl ToolRegistry for MockRegistry {
    fn definitions(&self) -> &[ToolDefinition] {
        &self.cached_definitions
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

pub(super) struct MockTool {
    def: ToolDefinition,
    response: Mutex<String>,
}

impl MockTool {
    pub(super) fn new(name: &str, response: &str) -> Self {
        Self {
            def: ToolDefinition {
                name: name.to_string().into(),
                description: format!("Mock {} tool", name).into(),
                parameters_schema: r#"{"type":"object"}"#.into(),
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

#[derive(Debug)]
struct PromptMutatingTool {
    def: ToolDefinition,
    next_prompt: Arc<Mutex<String>>,
    response: String,
}

impl PromptMutatingTool {
    fn new(name: &str, next_prompt: Arc<Mutex<String>>, response: &str) -> Self {
        Self {
            def: ToolDefinition {
                name: name.to_string().into(),
                description: format!("Prompt-mutating {} tool", name).into(),
                parameters_schema: r#"{"type":"object"}"#.into(),
            },
            next_prompt,
            response: response.to_string(),
        }
    }
}

impl crate::domain::tool::Tool for PromptMutatingTool {
    fn definition(&self) -> ToolDefinition {
        self.def.clone()
    }

    fn execute(
        &self,
        _arguments: &str,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<ToolResult, DomainError>> + Send + '_>>
    {
        let next_prompt = self.next_prompt.clone();
        let response = self.response.clone();
        Box::pin(async move {
            *next_prompt.lock().unwrap() = "System base\n\n## Active Development Workflow\nTemplate: Fix (fix)\nProgress: 1/6 steps complete.\nCURRENT STEP → 2. Write/update regression tests [RED]".to_string();
            Ok(ToolResult {
                content: response,
                is_error: false,
                image_blocks: vec![],
            })
        })
    }
}

// -----------------------------------------------------------------------
// Helper to build an AgentLoopImpl with mock components
// -----------------------------------------------------------------------

pub(super) fn make_agent(
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
        streaming: false,
        effort: None,
        system_prompt_provider: None,
        audit_log: None,
    });
    (agent, provider)
}

pub(super) fn text_response(content: &str) -> LlmResponse {
    LlmResponse {
        content: Some(content.to_string()),
        tool_calls: vec![],
        usage: Some(UsageInfo {
            prompt_tokens: 10,
            completion_tokens: 20,
            cache_read_tokens: None,
            cache_write_tokens: None,
            cost: None,
        }),
        stop_reason: None,
        thinking_blocks: vec![],
    }
}

pub(super) fn tool_call_response(name: &str, args: &str) -> LlmResponse {
    LlmResponse {
        content: None,
        tool_calls: vec![ToolCall {
            id: format!("call_{}", name),
            name: name.to_string(),
            arguments: args.to_string(),
        }],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
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
    let names: Vec<&str> = defs.iter().map(|d| d.name.as_ref()).collect();
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
            stop_reason: None,
            thinking_blocks: vec![],
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
        streaming: false,
        effort: None,
        system_prompt_provider: None,
        audit_log: None,
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

#[tokio::test]
async fn test_system_prompt_provider_is_refreshed_before_each_llm_turn() {
    let prompts = Arc::new(Mutex::new(
        "System base\n\n## Active Development Workflow\nMODE: SELECT TEMPLATE".to_string(),
    ));
    let provider = Arc::new(MockProvider::new(vec![
        tool_call_response("advance_workflow", r#"{}"#),
        text_response("done"),
    ]));
    let mut registry = MockRegistry::new();
    registry.register(Arc::new(PromptMutatingTool::new(
        "advance_workflow",
        prompts.clone(),
        "ok",
    )));
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
        streaming: false,
        effort: None,
        system_prompt_provider: Some(Arc::new({
            let prompts = prompts.clone();
            move || prompts.lock().unwrap().clone()
        })),
        audit_log: None,
    });

    let mut messages = vec![Message::system("stale prompt"), Message::user("advance")];
    let result = agent.run_loop(&mut messages).await.unwrap();

    assert_eq!(result.response, "done");
    assert_eq!(
        provider.seen_system_prompts(),
        vec![
            "System base\n\n## Active Development Workflow\nMODE: SELECT TEMPLATE"
                .to_string(),
            "System base\n\n## Active Development Workflow\nTemplate: Fix (fix)\nProgress: 1/6 steps complete.\nCURRENT STEP → 2. Write/update regression tests [RED]"
                .to_string(),
        ]
    );
}

#[test]
fn test_refresh_dynamic_system_prompt_inserts_before_manifest() {
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: Arc::new(MockProvider::new(vec![])),
        tool_registry: Box::new(MockRegistry::new()),
        model: "test-model".to_string(),
        max_tokens: 1024,
        temperature: 0.7,
        spill_store: None,
        session_key: String::new(),
        context_collapse_after_turns: u32::MAX,
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        system_prompt_provider: Some(Arc::new(|| "live prompt".to_string())),
        audit_log: None,
    });
    let mut manifest = Message::system("[Session memory: 1 spilled entry]");
    manifest.is_manifest = true;
    manifest.is_pinned = true;
    let mut messages = vec![manifest, Message::user("hello")];

    agent.refresh_dynamic_system_prompt(&mut messages);

    assert_eq!(messages[0].content, "live prompt");
    assert!(!messages[0].is_manifest);
    assert!(messages[1].is_manifest);
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
        streaming: false,
        effort: None,
        system_prompt_provider: None,
        audit_log: None,
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
        ..
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
    if let Some(crate::domain::agent::AgentProgressEvent::ToolStarted {
        name, arguments, ..
    }) = fired.iter().find(|e| {
        matches!(
            e,
            crate::domain::agent::AgentProgressEvent::ToolStarted { .. }
        )
    }) {
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

// --- #214: tool_count() on ToolRegistry trait ---

#[tokio::test]
async fn test_tool_count_on_registry_trait() {
    let mut registry = MockRegistry::new();
    registry.register(Arc::new(MockTool::new("bash", "")));
    registry.register(Arc::new(MockTool::new("read", "")));
    let trait_reg: &dyn ToolRegistry = &registry;
    assert_eq!(trait_reg.tool_count(), 2);
}

#[tokio::test]
async fn test_tool_count_empty() {
    let registry = MockRegistry::new();
    let trait_reg: &dyn ToolRegistry = &registry;
    assert_eq!(trait_reg.tool_count(), 0);
}

// --- #318: tool_call_id in ToolStarted/ToolFinished progress events ---

#[tokio::test]
async fn test_progress_callback_tool_started_includes_tool_call_id() {
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
    if let Some(crate::domain::agent::AgentProgressEvent::ToolStarted {
        tool_call_id, name, ..
    }) = fired.iter().find(|e| {
        matches!(
            e,
            crate::domain::agent::AgentProgressEvent::ToolStarted { .. }
        )
    }) {
        assert_eq!(name, "bash");
        assert_eq!(
            tool_call_id, "call_bash",
            "expected tool_call_id 'call_bash', got '{tool_call_id}'"
        );
    } else {
        panic!("expected ToolStarted event, got: {:?}", *fired);
    }
}

#[tokio::test]
async fn test_progress_callback_tool_finished_includes_tool_call_id() {
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
    if let Some(crate::domain::agent::AgentProgressEvent::ToolFinished {
        tool_call_id, name, ..
    }) = fired.iter().find(|e| {
        matches!(
            e,
            crate::domain::agent::AgentProgressEvent::ToolFinished { .. }
        )
    }) {
        assert_eq!(name, "bash");
        assert_eq!(
            tool_call_id, "call_bash",
            "expected tool_call_id 'call_bash', got '{tool_call_id}'"
        );
    } else {
        panic!("expected ToolFinished event, got: {:?}", *fired);
    }
}

#[tokio::test]
async fn retries_retryable_provider_failures_before_returning_success() {
    let provider = Arc::new(MockProvider::new_results(vec![
        Err(DomainError::Provider(
            "HTTP 503 Service Unavailable".to_string(),
        )),
        Ok(text_response("recovered")),
    ]));
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: provider.clone(),
        tool_registry: Box::new(MockRegistry::default()),
        model: "test".into(),
        max_tokens: 1024,
        temperature: 0.0,
        spill_store: None,
        session_key: "retry-test".into(),
        context_collapse_after_turns: context_pruning::COLLAPSE_DISABLED,
        max_context_tokens: 100_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        system_prompt_provider: None,
        audit_log: None,
    })
    .with_max_tool_iterations(1);

    let mut messages = vec![Message::user("hello")];
    let result = agent.process(&mut messages).await.unwrap();

    assert_eq!(result.response, "recovered");
    assert_eq!(provider.request_count(), 2);
}

#[tokio::test]
async fn retries_streaming_provider_failures_before_any_output() {
    let provider = Arc::new(MockStreamingProvider::new(vec![
        vec![crate::domain::provider::StreamEvent::Error(
            "HTTP 503 from Codex: connection refused".to_string(),
        )],
        vec![crate::domain::provider::StreamEvent::Done(text_response(
            "stream recovered",
        ))],
    ]));
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: provider.clone(),
        tool_registry: Box::new(MockRegistry::default()),
        model: "test".into(),
        max_tokens: 1024,
        temperature: 0.0,
        spill_store: None,
        session_key: "stream-retry-test".into(),
        context_collapse_after_turns: context_pruning::COLLAPSE_DISABLED,
        max_context_tokens: 100_000,
        progress_callback: None,
        streaming: true,
        effort: None,
        system_prompt_provider: None,
        audit_log: None,
    })
    .with_max_tool_iterations(1);

    let mut messages = vec![Message::user("hello")];
    let result = agent.process(&mut messages).await.unwrap();

    assert_eq!(result.response, "stream recovered");
    assert_eq!(provider.request_count(), 2);
}

#[tokio::test]
async fn does_not_retry_streaming_provider_failures_after_output() {
    let provider = Arc::new(MockStreamingProvider::new(vec![vec![
        crate::domain::provider::StreamEvent::TextDelta("partial".to_string()),
        crate::domain::provider::StreamEvent::Error("HTTP 503 from Codex".to_string()),
    ]]));
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: provider.clone(),
        tool_registry: Box::new(MockRegistry::default()),
        model: "test".into(),
        max_tokens: 1024,
        temperature: 0.0,
        spill_store: None,
        session_key: "stream-no-retry-test".into(),
        context_collapse_after_turns: context_pruning::COLLAPSE_DISABLED,
        max_context_tokens: 100_000,
        progress_callback: None,
        streaming: true,
        effort: None,
        system_prompt_provider: None,
        audit_log: None,
    })
    .with_max_tool_iterations(1);

    let mut messages = vec![Message::user("hello")];
    let err = agent.process(&mut messages).await.unwrap_err().to_string();

    assert!(err.contains("HTTP 503 from Codex"), "{err}");
    assert_eq!(provider.request_count(), 1);
}

#[tokio::test]
async fn provider_context_limit_errors_are_actionable() {
    let provider = Arc::new(MockProvider::new_results(vec![Err(DomainError::Provider(
        "HTTP 400 from OpenAI: maximum context length is 100000 tokens; requested 100001 tokens"
            .to_string(),
    ))]));
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(MockRegistry::default()),
        model: "test".into(),
        max_tokens: 8192,
        temperature: 0.0,
        spill_store: None,
        session_key: "limit-test".into(),
        context_collapse_after_turns: context_pruning::COLLAPSE_DISABLED,
        max_context_tokens: 100_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        system_prompt_provider: None,
        audit_log: None,
    })
    .with_max_tool_iterations(1);

    let mut messages = vec![Message::user("hello")];
    let err = agent.process(&mut messages).await.unwrap_err().to_string();

    assert!(
        err.to_ascii_lowercase().contains("context/output limit"),
        "{err}"
    );
    assert!(err.contains("reducing prompt history"), "{err}");
    assert!(err.contains("max output tokens"), "{err}");
}
