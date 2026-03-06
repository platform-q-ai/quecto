/// Spill-to-disk and tool_call_id tests for the agent loop.
///
/// Split from `agent_loop_tests.rs` to keep files within the 750-line limit.
use super::*;
use crate::domain::message::{LlmResponse, Message, Role, ToolCall, UsageInfo};
use crate::domain::session::{ContextSpillStore, SpillEntry};
use crate::domain::tool::{Tool, ToolDefinition, ToolRegistry, ToolResult};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

// Re-use test helpers from agent_loop_tests via the parent module.
// (They're in the same `tests` cfg scope since both files are
//  `#[path = "..."]` modules under `agent_loop.rs`.)

// ─── Mock infrastructure (duplicated minimal set needed for these tests) ──────

#[derive(Debug)]
struct SpillMockProvider {
    responses: Mutex<Vec<LlmResponse>>,
}
impl SpillMockProvider {
    fn new(responses: Vec<LlmResponse>) -> Self {
        Self {
            responses: Mutex::new(responses),
        }
    }
}
impl crate::domain::provider::LlmProvider for SpillMockProvider {
    fn chat(
        &self,
        request: crate::domain::provider::ChatRequest<'_>,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<LlmResponse, crate::domain::error::DomainError>> + Send + '_,
        >,
    > {
        let _ = request;
        let resp = self.responses.lock().unwrap().remove(0);
        Box::pin(async move { Ok(resp) })
    }
    fn name(&self) -> &str {
        "spill-mock"
    }
}

#[derive(Debug)]
struct SpillMockTool {
    name: String,
    output: String,
}
impl Tool for SpillMockTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name.clone().into(),
            description: "mock".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }
    }
    fn execute(
        &self,
        _args: &str,
    ) -> Pin<
        Box<dyn Future<Output = Result<ToolResult, crate::domain::error::DomainError>> + Send + '_>,
    > {
        let out = self.output.clone();
        Box::pin(async move {
            Ok(ToolResult {
                content: out,
                is_error: false,
                image_blocks: vec![],
            })
        })
    }
}

struct SpillMockRegistry {
    tools: std::collections::HashMap<String, Arc<dyn Tool>>,
    definitions: Vec<ToolDefinition>,
}
impl SpillMockRegistry {
    fn new() -> Self {
        Self {
            tools: std::collections::HashMap::new(),
            definitions: Vec::new(),
        }
    }
    fn register(&mut self, tool: Arc<dyn Tool>) {
        let name = tool.definition().name.to_string();
        self.definitions.push(tool.definition());
        self.tools.insert(name, tool);
    }
}
impl ToolRegistry for SpillMockRegistry {
    fn definitions(&self) -> &[ToolDefinition] {
        &self.definitions
    }
    fn execute(
        &self,
        name: &str,
        args: &str,
    ) -> Pin<
        Box<dyn Future<Output = Result<ToolResult, crate::domain::error::DomainError>> + Send + '_>,
    > {
        let tool = self.tools.get(name).cloned();
        let name = name.to_string();
        let args = args.to_string();
        Box::pin(async move {
            match tool {
                Some(t) => t.execute(&args).await,
                None => Err(crate::domain::error::DomainError::Tool(format!(
                    "unknown: {name}"
                ))),
            }
        })
    }
}

fn text_resp(content: &str) -> LlmResponse {
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
    }
}

fn tool_call_resp(name: &str, args: &str) -> LlmResponse {
    LlmResponse {
        content: None,
        tool_calls: vec![ToolCall {
            id: format!("call_{name}"),
            name: name.to_string(),
            arguments: args.to_string(),
        }],
        usage: None,
        stop_reason: None,
    }
}

/// Mock spill store that records appended entries.
#[derive(Debug, Default)]
struct MockSpillStore {
    entries: Mutex<Vec<SpillEntry>>,
}

impl ContextSpillStore for MockSpillStore {
    fn append(
        &self,
        _session_key: &str,
        entry: &SpillEntry,
    ) -> Pin<Box<dyn Future<Output = Result<(), crate::domain::error::DomainError>> + Send + '_>>
    {
        self.entries.lock().unwrap().push(entry.clone());
        Box::pin(async { Ok(()) })
    }

    fn recall(
        &self,
        _session_key: &str,
        _id: &str,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<Option<SpillEntry>, crate::domain::error::DomainError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async { Ok(None) })
    }

    fn list_entries(
        &self,
        _session_key: &str,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        Vec<crate::domain::session::SpillIndex>,
                        crate::domain::error::DomainError,
                    >,
                > + Send
                + '_,
        >,
    > {
        Box::pin(async { Ok(vec![]) })
    }

    fn clear(
        &self,
        _session_key: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), crate::domain::error::DomainError>> + Send + '_>>
    {
        Box::pin(async { Ok(()) })
    }
}

// ─── Spill tests ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_spill_preserves_message_content_after_spill() {
    let spill_store = Arc::new(MockSpillStore::default());
    let provider = Arc::new(SpillMockProvider::new(vec![
        tool_call_resp("bash", r#"{"command":"echo hi"}"#),
        text_resp("done"),
    ]));
    let mut registry = SpillMockRegistry::new();
    registry.register(Arc::new(SpillMockTool {
        name: "bash".into(),
        output: "big output here".into(),
    }));

    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(registry),
        model: "test-model".to_string(),
        max_tokens: 1024,
        temperature: 0.7,
        spill_store: Some(spill_store.clone()),
        session_key: "test-session".to_string(),
        context_collapse_after_turns: u32::MAX,
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: false,
    });

    let mut messages = vec![Message::user("run it")];
    agent.run_loop(&mut messages).await.unwrap();

    let tool_msg = messages.iter().find(|m| m.role == Role::Tool).unwrap();
    assert_eq!(
        tool_msg.content, "big output here",
        "tool message content must be preserved after spill"
    );

    let entries = spill_store.entries.lock().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].content, "big output here");
}
