//! Contract tests for the `AgentLoop` port.
//!
//! The port contract:
//! - `info()` exposes the agent's configuration summary.
//! - `process(&mut messages)` drives one or more LLM turns, appends the
//!   final assistant response to `messages`, and returns an `AgentResult`.
//! - When the provider returns an `EndTurn` response with no tool calls,
//!   the loop runs exactly one iteration and `tool_iterations == 0`.
//!
//! We drive the real `AgentLoopImpl` orchestrator with inline stubs for
//! `LlmProvider` and `ToolRegistry` so the test doesn't depend on network
//! or filesystem state.

use quecto::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use quecto::domain::agent::AgentLoop;
use quecto::domain::error::DomainError;
use quecto::domain::message::{LlmResponse, Message, Role, StopReason};
use quecto::domain::provider::{ChatRequest, LlmProvider};
use quecto::domain::tool::{ToolDefinition, ToolRegistry, ToolResult};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

#[derive(Debug)]
struct TextProvider {
    reply: String,
}
impl LlmProvider for TextProvider {
    fn name(&self) -> &str {
        "text"
    }
    fn chat<'a>(
        &'a self,
        _req: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        let reply = self.reply.clone();
        Box::pin(async move {
            Ok(LlmResponse {
                content: Some(reply),
                tool_calls: vec![],
                usage: None,
                stop_reason: Some(StopReason::EndTurn),
                thinking_blocks: vec![],
            })
        })
    }
}

struct EmptyRegistry;
impl ToolRegistry for EmptyRegistry {
    fn definitions(&self) -> &[ToolDefinition] {
        &[]
    }
    fn execute(
        &self,
        name: &str,
        _args: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let n = name.to_string();
        Box::pin(async move { Err(DomainError::Tool(format!("no tool: {n}"))) })
    }
}

fn agent_loop(reply: &str) -> Arc<dyn AgentLoop> {
    Arc::new(AgentLoopImpl::new(AgentLoopConfig {
        provider: Arc::new(TextProvider {
            reply: reply.into(),
        }),
        tool_registry: Box::new(EmptyRegistry),
        model: "test-model".into(),
        max_tokens: 1000,
        temperature: 0.0,
        spill_store: None,
        session_key: "contract".into(),
        context_collapse_after_turns: 100,
        max_context_tokens: 100_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        system_prompt_provider: None,
        audit_log: None,
    }))
}

#[tokio::test]
async fn info_surfaces_tool_count() {
    let agent = agent_loop("hi");
    let info = agent.info();
    assert_eq!(info.tool_count, 0);
}

#[tokio::test]
async fn process_appends_assistant_response_and_reports_zero_tool_iterations() {
    let agent = agent_loop("hello there");
    let mut messages = vec![Message::user("hi")];

    let result = agent
        .process(&mut messages)
        .await
        .expect("process must succeed for a simple text reply");

    assert_eq!(result.response, "hello there");
    assert_eq!(
        result.tool_iterations, 0,
        "a non-tool reply must run zero tool iterations"
    );
    assert!(!result.iteration_limit_reached);

    // The loop must have appended the assistant reply to the message history.
    assert_eq!(messages.last().unwrap().role, Role::Assistant);
    assert_eq!(messages.last().unwrap().content, "hello there");
}

#[tokio::test]
async fn process_is_idempotent_within_a_fresh_loop() {
    // Two independent agent instances processing the same prompt must
    // produce the same response shape (given the same provider behaviour).
    let a = agent_loop("same");
    let b = agent_loop("same");
    let mut ma = vec![Message::user("x")];
    let mut mb = vec![Message::user("x")];
    let ra = a.process(&mut ma).await.unwrap();
    let rb = b.process(&mut mb).await.unwrap();
    assert_eq!(ra.response, rb.response);
    assert_eq!(ra.tool_iterations, rb.tool_iterations);
}
