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
use quecto::domain::audit::{AuditEvent, AuditSink};
use quecto::domain::error::DomainError;
use quecto::domain::message::{LlmResponse, Message, Role, StopReason};
use quecto::domain::provider::{ChatRequest, LlmProvider};
use quecto::domain::tool::{
    RuntimeToolLifecycleRegistry, SessionAwareTools, ToolCatalog, ToolDefinition, ToolExecutor,
    ToolResult,
};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

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
impl ToolCatalog for EmptyRegistry {
    fn definitions(&self) -> &[ToolDefinition] {
        &[]
    }
}

impl ToolExecutor for EmptyRegistry {
    fn execute(
        &self,
        name: &str,
        _args: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let n = name.to_string();
        Box::pin(async move { Err(DomainError::Tool(format!("no tool: {n}"))) })
    }
}

impl RuntimeToolLifecycleRegistry for EmptyRegistry {}

impl SessionAwareTools for EmptyRegistry {}

impl quecto::domain::tool::ToolPolicyMutator for EmptyRegistry {}

impl quecto::domain::tool::ToolRegistry for EmptyRegistry {}

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
        context_collapse_after_tool_calls: 100,
        max_context_tokens: 100_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
        tool_profile_context: quecto::domain::tool::ToolProfileContext::Parent,
    }))
}

/// A provider that always fails terminally with a fixed error body.
#[derive(Debug)]
struct FailingProvider {
    body: String,
}
impl LlmProvider for FailingProvider {
    fn name(&self) -> &str {
        "failprov"
    }
    fn chat<'a>(
        &'a self,
        _req: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        let body = self.body.clone();
        Box::pin(async move { Err(DomainError::Provider(body)) })
    }
}

/// An audit sink that records every emitted event for inspection.
#[derive(Default)]
struct RecordingSink {
    events: Mutex<Vec<AuditEvent>>,
}
impl AuditSink for RecordingSink {
    fn emit(
        &self,
        _turn: u32,
        event: AuditEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        self.events.lock().unwrap().push(event);
        Box::pin(async { Ok(()) })
    }
}

fn failing_agent_loop(body: &str, sink: Arc<dyn AuditSink>) -> Arc<dyn AgentLoop> {
    Arc::new(AgentLoopImpl::new(AgentLoopConfig {
        provider: Arc::new(FailingProvider { body: body.into() }),
        tool_registry: Box::new(EmptyRegistry),
        model: "test-model".into(),
        max_tokens: 1000,
        temperature: 0.0,
        spill_store: None,
        session_key: "contract".into(),
        context_collapse_after_tool_calls: 100,
        max_context_tokens: 100_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: Some(sink),
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
        tool_profile_context: quecto::domain::tool::ToolProfileContext::Parent,
    }))
}

#[tokio::test]
async fn terminal_provider_failure_persists_full_redacted_body_via_audit() {
    // A long body well past any TUI/preview truncation cap, with a planted
    // secret. The audit ProviderError record must keep the WHOLE body and
    // redact the secret (#937 AC1/AC3/AC5).
    let secret = "sk-abc123SECRETvalue";
    let filler = "y".repeat(4000);
    let body = format!(
        r#"HTTP 400: {{"error":{{"type":"invalid_request_error","key":"{secret}","message":"server hated this {filler}"}}}}"#
    );

    let sink = Arc::new(RecordingSink::default());
    let agent = failing_agent_loop(&body, sink.clone());
    let mut messages = vec![Message::user("hi")];

    let result = agent.process(&mut messages).await;
    assert!(result.is_err(), "a terminal provider failure must error");

    let events = sink.events.lock().unwrap();
    let provider_errors: Vec<&AuditEvent> = events
        .iter()
        .filter(|e| matches!(e, AuditEvent::ProviderError { .. }))
        .collect();

    assert_eq!(
        provider_errors.len(),
        1,
        "exactly one ProviderError per terminal failure (AC4); got {}",
        provider_errors.len()
    );

    let AuditEvent::ProviderError {
        provider,
        class,
        http_status,
        body: persisted,
    } = provider_errors[0]
    else {
        unreachable!()
    };

    assert_eq!(provider, "failprov", "provider name captured");
    assert_eq!(
        *class,
        quecto::domain::provider_error::ProviderErrorClass::Client,
        "classified error class captured"
    );
    assert_eq!(*http_status, Some(400), "http status captured when known");
    // Full untruncated body retained except for the redacted secret span.
    assert!(
        persisted.len() > 4000,
        "body must be the full untruncated error (len={})",
        persisted.len()
    );
    assert!(
        persisted.contains(&filler),
        "the complete body must survive, not a truncated preview"
    );
    assert!(
        !persisted.contains(secret),
        "planted secret must be redacted: {persisted}"
    );
    assert!(
        persisted.contains("[REDACTED]"),
        "redaction marker must be present: {persisted}"
    );

    // Security (#939 review): the secret must be absent from the serialized
    // JSON of *every* emitted event, not just the ProviderError. An earlier
    // version also emitted an unredacted generic Error carrying the same body,
    // leaking the secret on the adjacent line; that event is now dropped.
    for ev in events.iter() {
        let json = serde_json::to_string(ev).unwrap();
        assert!(
            !json.contains(secret),
            "no emitted event may contain the unredacted secret: {json}"
        );
    }
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
