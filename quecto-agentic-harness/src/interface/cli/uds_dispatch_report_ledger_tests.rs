//! #2235 review: what `get_message` recovers by id carries the durable
//! ordinal the turn's save stamped — on a turn cancelled during a tool call
//! (its synthetic `aborted` result), a completed turn and a failed one. The
//! ledger's full copies are taken before the save stamps the live messages,
//! so they must be restamped after it.
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use super::super::fixture_tests::Fixture;
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::application::providers::ports::{ChatRequest, LlmProvider};
use crate::application::tools::ports::Tool;
use crate::domain::error::DomainError;
use crate::domain::message::{LlmResponse, Role, ToolCall};
use crate::domain::tool::{ToolDefinition, ToolResult};
use crate::interface::cli::uds_cancel::{CancelHandle, fire_cancel};
use crate::interface::uds::sessions::recover_message_controller::GetMessageFields;

type Reply = Result<LlmResponse, DomainError>;

#[derive(Debug)]
struct Scripted(Mutex<Vec<Reply>>);

impl LlmProvider for Scripted {
    fn name(&self) -> &str {
        "scripted"
    }
    fn chat(
        &self,
        _: ChatRequest<'_>,
    ) -> Pin<Box<dyn std::future::Future<Output = Reply> + Send + '_>> {
        let reply = self.0.lock().unwrap().remove(0);
        Box::pin(async move { reply })
    }
}

/// A tool that cancels the running turn and then never returns.
struct CancelsMidCall(CancelHandle);

impl Tool for CancelsMidCall {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "blocker".to_string().into(),
            description: "cancels the turn mid-call".to_string().into(),
            parameters_schema: r#"{"type":"object"}"#.to_string().into(),
        }
    }
    fn execute(
        &self,
        _: &str,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<ToolResult, DomainError>> + Send + '_>>
    {
        fire_cancel(&self.0);
        Box::pin(std::future::pending())
    }
}

/// A tool that answers at once.
struct Echo;

impl Tool for Echo {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "echo".to_string().into(),
            description: "answers at once".to_string().into(),
            parameters_schema: r#"{"type":"object"}"#.to_string().into(),
        }
    }
    fn execute(
        &self,
        _: &str,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<ToolResult, DomainError>> + Send + '_>>
    {
        Box::pin(async {
            Ok(ToolResult {
                content: "echoed".into(),
                is_error: false,
                image_blocks: vec![],
                delivery_metadata: None,
            })
        })
    }
}

fn reply(content: Option<&str>, call: Option<&str>) -> Reply {
    Ok(LlmResponse {
        content: content.map(str::to_string),
        tool_calls: call
            .map(|name| ToolCall {
                id: format!("call_{name}"),
                name: name.to_string(),
                arguments: "{}".to_string(),
            })
            .into_iter()
            .collect(),
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    })
}

fn fixture(replies: Vec<Reply>) -> Fixture {
    let mut fx = Fixture::new();
    let mut registry = crate::infrastructure::tools::registry::ToolRegistryImpl::new();
    registry.register(Arc::new(CancelsMidCall(fx.cancel.clone())));
    registry.register(Arc::new(Echo));
    fx.set_agent(AgentLoopImpl::new(AgentLoopConfig {
        provider: Arc::new(Scripted(Mutex::new(replies))),
        tool_registry: Box::new(registry),
        model: "stub".into(),
        max_tokens: 100,
        temperature: 0.0,
        retention: None,
        session_key: "cli:test".into(),
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_marks: Default::default(),
        model_context_window: None,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    }));
    fx
}

async fn follow_up(fx: &mut Fixture, task: &str) {
    let mut ctx = fx.ctx();
    super::super::handle_follow_up(&mut ctx, Some("task"), "follow_up", task.into()).await;
}

/// Every message of the live transcript, recovered by id from the ledger
/// (no live fallback), carries the ordinal the live message carries.
async fn assert_recovered_ordinals_match(fx: &Fixture) {
    let handles = fx.sessions.read_handles();
    for live in &fx.messages {
        let id = live.id().to_string();
        let fields = GetMessageFields {
            message_id: &id,
            tool_call_id: None,
            offset: None,
            thinking_offset: None,
            limit: None,
        };
        let recovered = handles.recover_message.recover(fields, &[]).await;
        let Ok(crate::application::sessions::dto::RecoveredContent::Message { message, .. }) =
            recovered
        else {
            panic!("{:?} {id} is not recoverable: {recovered:?}", live.role);
        };
        assert!(live.ordinal.is_some(), "{:?} was saved", live.role);
        assert_eq!(
            message.ordinal, live.ordinal,
            "{:?} {:?}",
            live.role, live.content
        );
    }
}

#[tokio::test]
async fn a_turn_cancelled_during_a_tool_call_recovers_its_aborted_result_with_its_ordinal() {
    let mut fx = fixture(vec![reply(None, Some("blocker"))]);
    follow_up(&mut fx, "task").await;
    let aborted = fx
        .messages
        .iter()
        .find(|m| m.role == Role::Tool && m.tool_call_id.as_deref() == Some("call_blocker"))
        .expect("the unanswered call got a synthetic result");
    assert!(aborted.is_error);
    assert_recovered_ordinals_match(&fx).await;
}

#[tokio::test]
async fn a_completed_turn_recovers_every_message_with_its_ordinal() {
    let mut fx = fixture(vec![reply(Some("the report"), None)]);
    follow_up(&mut fx, "task").await;
    assert!(fx.messages.iter().any(|m| m.content == "the report"));
    assert_recovered_ordinals_match(&fx).await;
}

#[tokio::test]
async fn a_turn_that_fails_after_a_tool_call_recovers_every_message_with_its_ordinal() {
    let mut fx = fixture(vec![
        reply(None, Some("echo")),
        Err(DomainError::Provider("down".into())),
    ]);
    follow_up(&mut fx, "task").await;
    let roles: Vec<Role> = fx.messages.iter().map(|m| m.role.clone()).collect();
    assert_eq!(
        roles,
        [Role::User, Role::Assistant, Role::Tool],
        "{:?}",
        fx.messages
    );
    assert_recovered_ordinals_match(&fx).await;
}
