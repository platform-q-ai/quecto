// Agent loop implementation: orchestrates LLM calls and tool execution.
// Depends on: domain::LlmProvider, domain::Tool, infrastructure::tools::ToolRegistry

use std::pin::Pin;
use std::sync::Arc;

use crate::domain::agent::{AgentInfo, AgentLoop, AgentResult};
use crate::domain::error::DomainError;
use crate::domain::message::{Message, Role};
use crate::domain::provider::{ChatRequest, LlmProvider};
use crate::domain::tool::ToolRegistry;

/// Default maximum tool iterations before the loop is forcibly stopped.
const DEFAULT_MAX_TOOL_ITERATIONS: u32 = 25;

/// Configuration for building an agent loop.
pub struct AgentLoopConfig {
    pub provider: Arc<dyn LlmProvider>,
    pub tool_registry: Box<dyn ToolRegistry>,
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,
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

    /// Run the LLM-tool loop.
    async fn run_loop(&self, messages: &mut Vec<Message>) -> Result<AgentResult, DomainError> {
        let tool_defs = self.tool_registry.definitions();
        let mut iterations: u32 = 0;

        loop {
            // Call the LLM
            let request = ChatRequest {
                messages,
                tools: &tool_defs,
                model: &self.model,
                max_tokens: self.max_tokens,
                temperature: self.temperature,
            };
            let response = self.provider.chat(request).await?;

            // If the LLM returned tool calls, execute them
            if !response.tool_calls.is_empty() {
                // Append the assistant message with tool calls
                messages.push(Message {
                    role: Role::Assistant,
                    content: response.content.clone().unwrap_or_default(),
                    tool_calls: response.tool_calls.clone(),
                    tool_call_id: None,
                });

                // Execute each tool call
                for tc in &response.tool_calls {
                    let tool_result = self.tool_registry.execute(&tc.name, &tc.arguments).await;

                    let (content, _is_error) = match tool_result {
                        Ok(tr) => (tr.content, tr.is_error),
                        Err(e) => (format!("Error: {}", e), true),
                    };

                    // Append tool result message
                    messages.push(Message {
                        role: Role::Tool,
                        content,
                        tool_calls: vec![],
                        tool_call_id: Some(tc.id.clone()),
                    });
                }

                iterations += 1;

                // Check iteration limit
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

                // Continue the loop — send tool results back to LLM
                continue;
            }

            // No tool calls — we have the final response
            let text = response.content.unwrap_or_default();

            // Append the final assistant message
            messages.push(Message {
                role: Role::Assistant,
                content: text.clone(),
                tool_calls: vec![],
                tool_call_id: None,
            });

            return Ok(AgentResult {
                response: text,
                tool_iterations: iterations,
                iteration_limit_reached: false,
            });
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
    use crate::domain::message::{LlmResponse, ToolCall, UsageInfo};
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
        let mut messages = vec![Message {
            role: Role::User,
            content: "Hi".to_string(),
            tool_calls: vec![],
            tool_call_id: None,
        }];
        let result = agent.run_loop(&mut messages).await.unwrap();
        assert_eq!(result.response, "Hello, world!");
        assert_eq!(result.tool_iterations, 0);
        assert!(!result.iteration_limit_reached);
    }

    #[tokio::test]
    async fn test_single_tool_call() {
        let (agent, _) = make_agent(
            vec![
                tool_call_response("read_file", r#"{"path":"notes.txt"}"#),
                text_response("Your notes say: Buy groceries"),
            ],
            vec![("read_file", "Buy groceries")],
        );
        let mut messages = vec![Message {
            role: Role::User,
            content: "What are my notes?".to_string(),
            tool_calls: vec![],
            tool_call_id: None,
        }];
        let result = agent.run_loop(&mut messages).await.unwrap();
        assert_eq!(result.response, "Your notes say: Buy groceries");
        assert_eq!(result.tool_iterations, 1);
    }

    #[tokio::test]
    async fn test_multiple_tool_calls_in_sequence() {
        let (agent, _) = make_agent(
            vec![
                tool_call_response("read_file", r#"{"path":"a.txt"}"#),
                tool_call_response("write_file", r#"{"path":"b.txt","content":"data"}"#),
                text_response("Done copying"),
            ],
            vec![("read_file", "file content"), ("write_file", "ok")],
        );
        let mut messages = vec![Message {
            role: Role::User,
            content: "Copy files".to_string(),
            tool_calls: vec![],
            tool_call_id: None,
        }];
        let result = agent.run_loop(&mut messages).await.unwrap();
        assert_eq!(result.response, "Done copying");
        assert_eq!(result.tool_iterations, 2);
    }

    #[tokio::test]
    async fn test_iteration_limit() {
        // LLM always returns tool calls — should stop at limit
        let responses: Vec<LlmResponse> = (0..10)
            .map(|i| tool_call_response("exec", &format!(r#"{{"cmd":"echo {}"}}"#, i)))
            .collect();
        let (agent, _) = make_agent(responses, vec![("exec", "output")]);
        let agent = agent.with_max_tool_iterations(3);

        let mut messages = vec![Message {
            role: Role::User,
            content: "Loop forever".to_string(),
            tool_calls: vec![],
            tool_call_id: None,
        }];
        let result = agent.run_loop(&mut messages).await.unwrap();
        assert!(result.iteration_limit_reached);
        assert_eq!(result.tool_iterations, 3);
        assert!(result.response.contains("limit"));
    }

    #[tokio::test]
    async fn test_tool_definitions_sent_to_llm() {
        let (agent, provider) = make_agent(
            vec![text_response("ok")],
            vec![("exec", ""), ("read_file", "")],
        );
        let mut messages = vec![Message {
            role: Role::User,
            content: "test".to_string(),
            tool_calls: vec![],
            tool_call_id: None,
        }];
        let _ = agent.run_loop(&mut messages).await.unwrap();
        let defs = provider.last_tool_defs();
        assert_eq!(defs.len(), 2);
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"exec"));
        assert!(names.contains(&"read_file"));
    }

    #[tokio::test]
    async fn test_agent_info() {
        let (agent, _) = make_agent(
            vec![],
            vec![("exec", ""), ("read_file", ""), ("write_file", "")],
        );
        let agent = agent.with_skill_count(2);
        let info = agent.info();
        assert_eq!(info.tool_count, 3);
        assert_eq!(info.skill_count, 2);
    }

    #[tokio::test]
    async fn test_messages_appended_during_loop() {
        let (agent, _) = make_agent(
            vec![
                tool_call_response("read_file", r#"{"path":"x"}"#),
                text_response("final"),
            ],
            vec![("read_file", "content")],
        );
        let mut messages = vec![Message {
            role: Role::User,
            content: "read".to_string(),
            tool_calls: vec![],
            tool_call_id: None,
        }];
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
        });
        let mut messages = vec![Message {
            role: Role::User,
            content: "use a tool".to_string(),
            tool_calls: vec![],
            tool_call_id: None,
        }];
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
