use super::*;
use crate::domain::message::{LlmResponse, Role, ToolCall, UsageInfo};
use crate::domain::tool::{
    Tool, ToolCatalog, ToolDefinition, ToolExecutor, ToolRegistry, ToolResult,
};
use std::sync::{Arc, Mutex};

#[derive(Debug)]
pub(super) struct MockProvider {
    responses: Mutex<Vec<Result<LlmResponse, DomainError>>>,
    /// Captured tool definitions from the last chat() call.
    last_tool_defs: Mutex<Vec<ToolDefinition>>,
    /// Captured `max_tokens` (effective output cap) from every chat() call (#935).
    seen_max_tokens: Mutex<Vec<u32>>,
    request_count: Mutex<usize>,
}

impl MockProvider {
    pub(super) fn new(responses: Vec<LlmResponse>) -> Self {
        Self {
            responses: Mutex::new(responses.into_iter().map(Ok).collect()),
            last_tool_defs: Mutex::new(vec![]),
            seen_max_tokens: Mutex::new(vec![]),
            request_count: Mutex::new(0),
        }
    }

    pub(super) fn new_results(responses: Vec<Result<LlmResponse, DomainError>>) -> Self {
        Self {
            responses: Mutex::new(responses),
            last_tool_defs: Mutex::new(vec![]),
            seen_max_tokens: Mutex::new(vec![]),
            request_count: Mutex::new(0),
        }
    }

    pub(super) fn seen_max_tokens(&self) -> Vec<u32> {
        self.seen_max_tokens.lock().unwrap().clone()
    }

    pub(super) fn request_count(&self) -> usize {
        *self.request_count.lock().unwrap()
    }

    pub(super) fn last_tool_defs(&self) -> Vec<ToolDefinition> {
        self.last_tool_defs.lock().unwrap().clone()
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
        *self.request_count.lock().unwrap() += 1;
        let events = self.responses.lock().unwrap().remove(0);
        let response = events.into_iter().find_map(|event| match event {
            crate::domain::provider::StreamEvent::Done(response) => Some(response),
            _ => None,
        });
        Box::pin(async move {
            response.ok_or_else(|| DomainError::Provider("missing done event".into()))
        })
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
        let max_tokens = request.max_tokens;
        self.seen_max_tokens.lock().unwrap().push(max_tokens);
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

#[derive(Default)]
pub(super) struct MockRegistry {
    pub(super) tools: Vec<Arc<dyn Tool>>,
    pub(super) cached_definitions: Vec<ToolDefinition>,
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

impl ToolCatalog for MockRegistry {
    fn definitions(&self) -> &[ToolDefinition] {
        &self.cached_definitions
    }
}

impl ToolExecutor for MockRegistry {
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

/// Baseline test config; override individual fields with functional-update
/// syntax (`AgentLoopConfig { field: ..., ..test_config(...) }`).
pub(super) fn test_config(
    provider: Arc<dyn crate::domain::provider::LlmProvider>,
    tool_registry: Box<dyn crate::domain::tool::ToolRegistry>,
) -> AgentLoopConfig {
    AgentLoopConfig {
        provider,
        tool_registry,
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
    }
}

pub(super) fn make_agent(
    responses: Vec<LlmResponse>,
    tools: Vec<(&str, &str)>,
) -> (AgentLoopImpl, Arc<MockProvider>) {
    let provider = Arc::new(MockProvider::new(responses));
    let mut registry = MockRegistry::new();
    for (name, response) in tools {
        registry.register(Arc::new(MockTool::new(name, response)));
    }
    let agent = AgentLoopImpl::new(test_config(provider.clone(), Box::new(registry)));
    (agent, provider)
}

fn empty_chat_request() -> ChatRequest<'static> {
    ChatRequest {
        messages: &[],
        tools: &[],
        model: "test",
        max_tokens: 1,
        temperature: 0.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    }
}

#[tokio::test]
async fn mock_providers_trait_surface_methods_are_invoked() {
    let streaming =
        MockStreamingProvider::new(vec![vec![crate::domain::provider::StreamEvent::Done(
            LlmResponse {
                content: Some("streamed".into()),
                tool_calls: vec![],
                usage: None,
                stop_reason: None,
                thinking_blocks: vec![],
            },
        )]]);
    assert_eq!(streaming.name(), "mock-streaming");
    assert!(streaming.as_any().is::<()>());
    assert_eq!(
        streaming
            .chat_stream(empty_chat_request())
            .await
            .unwrap()
            .content
            .as_deref(),
        Some("streamed")
    );

    let provider = MockProvider::new(vec![LlmResponse {
        content: Some("plain".into()),
        tool_calls: vec![],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    }]);
    assert_eq!(provider.name(), "mock");
    assert_eq!(
        provider
            .chat_stream(empty_chat_request())
            .await
            .unwrap()
            .content
            .as_deref(),
        Some("plain")
    );
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
            context_tokens: None,
            cost: None,
        }),
        stop_reason: None,
        thinking_blocks: vec![],
    }
}

#[derive(Clone, Copy)]
struct UsageFixture(u32, u32, u32, u32, u64);

fn text_response_with_usage(content: &str, fixture: UsageFixture) -> LlmResponse {
    LlmResponse {
        content: Some(content.to_string()),
        tool_calls: vec![],
        usage: Some(UsageInfo {
            prompt_tokens: fixture.0,
            completion_tokens: fixture.1,
            cache_read_tokens: Some(fixture.2),
            cache_write_tokens: Some(fixture.3),
            context_tokens: None,
            cost: Some(crate::domain::message::CostInfo {
                input_cost_micro_usd: 0,
                output_cost_micro_usd: 0,
                cache_read_cost_micro_usd: 0,
                cache_write_cost_micro_usd: 0,
                total_cost_micro_usd: fixture.4,
            }),
        }),
        stop_reason: None,
        thinking_blocks: vec![],
    }
}

fn tool_call_response_with_usage(name: &str, args: &str, fixture: UsageFixture) -> LlmResponse {
    LlmResponse {
        content: None,
        tool_calls: vec![ToolCall {
            id: format!("call_{}", name),
            name: name.to_string(),
            arguments: args.to_string(),
        }],
        usage: Some(UsageInfo {
            prompt_tokens: fixture.0,
            completion_tokens: fixture.1,
            cache_read_tokens: Some(fixture.2),
            cache_write_tokens: Some(fixture.3),
            context_tokens: None,
            cost: Some(crate::domain::message::CostInfo {
                input_cost_micro_usd: 0,
                output_cost_micro_usd: 0,
                cache_read_cost_micro_usd: 0,
                cache_write_cost_micro_usd: 0,
                total_cost_micro_usd: fixture.4,
            }),
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
async fn test_agent_result_accumulates_usage_cache_and_cost_across_llm_calls() {
    let (agent, _) = make_agent(
        vec![
            tool_call_response_with_usage(
                "read",
                r#"{"path":"x"}"#,
                UsageFixture(10, 2, 3, 4, 1_000),
            ),
            text_response_with_usage("final", UsageFixture(20, 5, 6, 7, 2_500)),
        ],
        vec![("read", "content")],
    );
    let mut messages = vec![Message::user("read")];

    let result = agent.run_loop(&mut messages).await.unwrap();

    assert_eq!(result.response, "final");
    assert_eq!(result.input_tokens, 20);
    assert_eq!(result.output_tokens, 7);
    assert_eq!(result.billed_input_tokens, 30);
    assert_eq!(result.billed_output_tokens, 7);
    assert_eq!(result.cache_read_tokens, 9);
    assert_eq!(result.cache_write_tokens, 11);
    assert_eq!(result.cost_micro_usd, 3_500);
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
    let info = agent.info();
    assert_eq!(info.tool_count, 3);
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
    let agent = AgentLoopImpl::new(test_config(provider, Box::new(registry)));
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
        progress_callback: Some(callback),
        ..test_config(provider.clone(), Box::new(registry))
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

#[path = "agent_loop_935_tests.rs"]
mod clamp_max_tokens_tests;
#[path = "agent_loop_context_tokens_tests.rs"]
mod context_tokens_tests;
#[path = "agent_loop_progress_tests.rs"]
mod progress_tests;
#[path = "agent_loop_931_tests.rs"]
mod retry_malformed_tests;

#[test]
fn new_threads_context_knobs_and_model_window_into_observable_budget() {
    let provider = Arc::new(MockProvider::new(vec![]));
    let registry = MockRegistry::new();
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        pin_recent_turns: 7,
        context_collapse_after_messages: 11,
        max_context_tokens: 10_000,
        model_context_window: Some(4_096),
        max_tokens: 512,
        ..test_config(provider, Box::new(registry))
    });
    assert_eq!(agent.context_knob_snapshot(), (7, 11));
    assert_eq!(agent.max_context_tokens(), 4_096);
    let debug = format!("{agent:?}");
    assert!(debug.contains("test-model"));
    assert!(debug.contains("max_tool_iterations"));
    assert!(!debug.contains("temperature"));
}

#[test]
fn take_durable_prefix_dirty_consumes_latch_once() {
    let (agent, _) = make_agent(vec![], vec![]);
    assert!(!agent.take_durable_prefix_dirty());
    agent.latch_durable_prefix_dirty();
    assert!(agent.take_durable_prefix_dirty());
    assert!(!agent.take_durable_prefix_dirty());
}

#[tokio::test]
async fn mock_provider_trait_surface_chat_stream_defaults_to_chat() {
    let provider = MockProvider::new(vec![text_response("stream default")]);
    let messages = [];
    let tools = [];
    let request = ChatRequest {
        messages: &messages,
        tools: &tools,
        model: "test-model",
        max_tokens: 9,
        temperature: 0.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let response = provider.chat_stream(request).await.unwrap();
    assert_eq!(response.content.as_deref(), Some("stream default"));
    assert_eq!(provider.request_count(), 1);
}

#[tokio::test]
async fn mock_streaming_provider_trait_surface_chat_and_incremental() {
    let provider =
        MockStreamingProvider::new(vec![vec![crate::domain::provider::StreamEvent::Done(
            text_response("done"),
        )]]);
    let messages = [];
    let tools = [];
    let request = ChatRequest {
        messages: &messages,
        tools: &tools,
        model: "test-model",
        max_tokens: 9,
        temperature: 0.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    assert_eq!(provider.name(), "mock-streaming");
    assert!(provider.as_any().is::<()>());
    let mut rx = provider.chat_stream_incremental(request).await;
    assert!(matches!(
        rx.recv().await,
        Some(crate::domain::provider::StreamEvent::Done(_))
    ));
    assert_eq!(provider.request_count(), 1);

    let provider =
        MockStreamingProvider::new(vec![vec![crate::domain::provider::StreamEvent::Done(
            text_response("chat done"),
        )]]);
    let messages = [];
    let tools = [];
    let request = ChatRequest {
        messages: &messages,
        tools: &tools,
        model: "test-model",
        max_tokens: 9,
        temperature: 0.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let response = provider.chat(request).await.unwrap();
    assert_eq!(response.content.as_deref(), Some("chat done"));
}
