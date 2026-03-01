// Agent loop implementation: orchestrates LLM calls and tool execution.
// Depends on: domain::LlmProvider, domain::Tool, infrastructure::tools::ToolRegistry

use std::pin::Pin;
use std::sync::Arc;

use crate::application::context_pruning;
use crate::domain::agent::{AgentInfo, AgentLoop, AgentResult};
use crate::domain::error::DomainError;
use crate::domain::message::{LlmResponse, Message, ToolCall};
use crate::domain::provider::{ChatRequest, LlmProvider};
use crate::domain::session::{ContextSpillStore, SpillEntry};
use crate::domain::tool::ToolRegistry;

/// Default maximum tool iterations before the loop is forcibly stopped.
const DEFAULT_MAX_TOOL_ITERATIONS: u32 = 999_999;

/// Configuration for building an agent loop.
pub struct AgentLoopConfig {
    pub provider: Arc<dyn LlmProvider>,
    pub tool_registry: Box<dyn ToolRegistry>,
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,
    pub spill_store: Option<Arc<dyn ContextSpillStore>>,
    pub session_key: String,
    pub context_collapse_after_turns: u32,
    pub max_context_tokens: usize,
}

/// Concrete implementation of the agent loop.
pub struct AgentLoopImpl {
    provider: Arc<dyn LlmProvider>,
    tool_registry: Box<dyn ToolRegistry>,
    model: String,
    max_tokens: u32,
    temperature: f32,
    max_tool_iterations: u32,
    skill_count: usize,
    spill_store: Option<Arc<dyn ContextSpillStore>>,
    session_key: String,
    context_collapse_after_turns: u32,
    max_context_tokens: usize,
}

impl std::fmt::Debug for AgentLoopImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentLoopImpl")
            .field("provider", &self.provider.name())
            .field("model", &self.model)
            .field("max_tool_iterations", &self.max_tool_iterations)
            .finish()
    }
}

/// Arguments for building a tool result message (avoids clippy 5-arg limit).
struct ToolMessageArgs<'a> {
    tc: &'a ToolCall,
    content: String,
    image_blocks: Vec<crate::domain::tool::ImageBlock>,
    spill_id: String,
}

impl AgentLoopImpl {
    pub fn new(config: AgentLoopConfig) -> Self {
        Self {
            provider: config.provider,
            tool_registry: config.tool_registry,
            model: config.model,
            max_tokens: config.max_tokens,
            temperature: config.temperature,
            max_tool_iterations: DEFAULT_MAX_TOOL_ITERATIONS,
            skill_count: 0,
            spill_store: config.spill_store,
            session_key: config.session_key,
            context_collapse_after_turns: config.context_collapse_after_turns,
            max_context_tokens: config.max_context_tokens,
        }
    }

    /// Set the maximum number of tool iterations (overrides default).
    pub fn with_max_tool_iterations(mut self, max: u32) -> Self {
        self.max_tool_iterations = max;
        self
    }

    /// Set the skill count (for startup info).
    pub fn with_skill_count(mut self, count: usize) -> Self {
        self.skill_count = count;
        self
    }

    async fn apply_context_pruning(
        &self,
        messages: &mut Vec<Message>,
        current_turn: u32,
        spills_dirty: bool,
    ) {
        // Collapse is disabled by default (COLLAPSE_DISABLED = u32::MAX).
        // Still available for users who explicitly lower the config value.
        let collapsed = if self.context_collapse_after_turns < context_pruning::COLLAPSE_DISABLED {
            context_pruning::collapse_old_tool_results(
                messages,
                current_turn,
                self.context_collapse_after_turns,
            )
        } else {
            0
        };
        let dropped = context_pruning::enforce_context_ceiling(messages, self.max_context_tokens);
        // Only rebuild manifest when spills have changed (new tool results spilled)
        if spills_dirty {
            if let Some(ref spill_store) = self.spill_store {
                context_pruning::update_spill_manifest(
                    messages,
                    spill_store.as_ref(),
                    &self.session_key,
                )
                .await;
            }
        }
        if collapsed > 0 || dropped > 0 {
            tracing::info!(
                target: "context_prune",
                collapsed,
                dropped,
                turn = current_turn,
                total_tokens = context_pruning::estimate_total_tokens(messages),
                "context pruned"
            );
        }
    }

    fn build_chat_request<'a>(
        &'a self,
        messages: &'a Vec<Message>,
        tool_defs: &'a [crate::domain::tool::ToolDefinition],
    ) -> ChatRequest<'a> {
        ChatRequest {
            messages,
            tools: tool_defs,
            model: &self.model,
            max_tokens: self.max_tokens,
            temperature: self.temperature,
        }
    }

    async fn execute_tool_calls_for_response(
        &self,
        messages: &mut Vec<Message>,
        current_turn: u32,
        response: &LlmResponse,
    ) {
        messages.push(Message::assistant(
            response.content.clone().unwrap_or_default(),
            response.tool_calls.clone(),
        ));

        for (idx, tc) in response.tool_calls.iter().enumerate() {
            let (content, image_blocks) = self.execute_single_tool_call(tc).await;
            let spill_id = format!("turn{}:{}:{}", current_turn, tc.name, idx);
            let mut tool_msg = self.build_tool_message(ToolMessageArgs {
                tc,
                content,
                image_blocks,
                spill_id,
            });
            tool_msg.turn = Some(current_turn);
            self.spill_tool_message(&mut tool_msg).await;
            messages.push(tool_msg);
        }
    }

    async fn execute_single_tool_call(
        &self,
        tc: &ToolCall,
    ) -> (String, Vec<crate::domain::tool::ImageBlock>) {
        let start = std::time::Instant::now();
        let tool_result = self.tool_registry.execute(&tc.name, &tc.arguments).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        let is_err = tool_result.is_err();
        let (content, image_blocks) = match tool_result {
            Ok(tr) => (tr.content, tr.image_blocks),
            Err(e) => (format!("Error: {}", e), vec![]),
        };

        tracing::info!(
            target: "tool_exec",
            tool_name = tc.name.as_str(),
            duration_ms,
            is_error = is_err,
            "tool executed"
        );
        (content, image_blocks)
    }

    fn build_tool_message(&self, args: ToolMessageArgs) -> Message {
        let mut tool_msg = Message::tool(args.tc.id.clone(), args.content);
        tool_msg.tool_name = Some(args.tc.name.clone());
        tool_msg.input_preview = Some(context_pruning::truncate_utf8_safe(&args.tc.arguments, 100));
        tool_msg.spill_id = Some(args.spill_id);
        tool_msg.image_blocks = args.image_blocks;
        tool_msg
    }

    async fn spill_tool_message(&self, tool_msg: &mut Message) {
        let Some(ref spill_store) = self.spill_store else {
            return;
        };

        let entry = SpillEntry {
            id: tool_msg.spill_id.clone().unwrap_or_default(),
            tool: tool_msg
                .tool_name
                .clone()
                .unwrap_or_else(|| "tool".to_string()),
            input_preview: tool_msg.input_preview.clone().unwrap_or_default(),
            tokens: context_pruning::estimate_tokens(&tool_msg.content),
            content: tool_msg.content.clone(),
        };
        if let Err(e) = spill_store.append(&self.session_key, &entry).await {
            tracing::warn!(target: "context_prune", error = %e, "failed to spill tool output");
        }
    }

    fn finalize_text_response(
        &self,
        messages: &mut Vec<Message>,
        response: LlmResponse,
        iterations: u32,
    ) -> AgentResult {
        let text = response.content.unwrap_or_default();
        messages.push(Message::assistant(text.clone(), vec![]));
        AgentResult {
            response: text,
            tool_iterations: iterations,
            iteration_limit_reached: false,
        }
    }

    /// Run the LLM-tool loop.
    async fn run_loop(&self, messages: &mut Vec<Message>) -> Result<AgentResult, DomainError> {
        let tool_defs = self.tool_registry.definitions();
        let mut iterations: u32 = 0;
        let mut current_turn: u32 = 1;
        // Track whether spills happened so we only rebuild manifest when needed.
        // Start true to build initial manifest from any prior session spills.
        let mut spills_dirty = true;

        loop {
            self.apply_context_pruning(messages, current_turn, spills_dirty)
                .await;

            let request = self.build_chat_request(messages, &tool_defs);
            let response = self.provider.chat(request).await?;

            if response.tool_calls.is_empty() {
                return Ok(self.finalize_text_response(messages, response, iterations));
            }

            self.execute_tool_calls_for_response(messages, current_turn, &response)
                .await;
            // Tool calls were executed and spilled — mark dirty for next iteration
            spills_dirty = self.spill_store.is_some();
            iterations += 1;
            current_turn += 1;

            if iterations >= self.max_tool_iterations {
                return Ok(AgentResult {
                    response: format!(
                        "Tool iteration limit ({}) reached. Stopping.",
                        self.max_tool_iterations
                    ),
                    tool_iterations: iterations,
                    iteration_limit_reached: true,
                });
            }
        }
    }
}

impl AgentLoop for AgentLoopImpl {
    fn process<'a>(
        &'a self,
        messages: &'a mut Vec<Message>,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<AgentResult, DomainError>> + Send + 'a>>
    {
        Box::pin(self.run_loop(messages))
    }

    fn info(&self) -> AgentInfo {
        AgentInfo {
            tool_count: self.tool_registry.definitions().len(),
            skill_count: self.skill_count,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::message::{LlmResponse, Role, ToolCall, UsageInfo};
    use crate::domain::tool::{ToolDefinition, ToolResult};
    use crate::infrastructure::tools::registry::ToolRegistryImpl;
    use std::sync::Mutex;

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
        let mut registry = ToolRegistryImpl::new();
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
        let (agent, provider) =
            make_agent(vec![text_response("ok")], vec![("bash", ""), ("read", "")]);
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
        let registry = ToolRegistryImpl::new(); // empty
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
}
