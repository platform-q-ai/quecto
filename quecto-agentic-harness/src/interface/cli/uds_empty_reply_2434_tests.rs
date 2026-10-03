//! #2434 over the UDS: a model reply to tool results with nothing in it
//! ends the run with `agent_end`, at once; an empty first reply to a prompt
//! and a reasoning-only reply at the output limit (#2124) still end it with
//! `agent_error`.
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use super::{EventSink, PromptOutcome, PromptRun, run_agent_message};
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::application::providers::ports::{ChatRequest, LlmProvider};
use crate::application::tools::ports::Tool;
use crate::domain::error::DomainError;
use crate::domain::message::{LlmResponse, Message, StopReason, ThinkingBlock, ToolCall};
use crate::domain::provider::StreamEvent;
use crate::domain::tool::{ToolDefinition, ToolResult};
use crate::interface::cli::uds_session::AgentSession;

/// Streams each scripted reply as its `Done` event, one per request.
#[derive(Debug)]
struct ScriptedStream {
    replies: Mutex<Vec<LlmResponse>>,
    requests: Mutex<usize>,
}

impl ScriptedStream {
    fn next(&self) -> LlmResponse {
        *self.requests.lock().unwrap() += 1;
        self.replies.lock().unwrap().remove(0)
    }
}

impl LlmProvider for ScriptedStream {
    fn route_order(&self) -> Vec<String> {
        vec![self.name().to_string()]
    }
    fn name(&self) -> &str {
        "scripted-2434"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn chat(
        &self,
        _request: ChatRequest<'_>,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<LlmResponse, DomainError>> + Send + '_>>
    {
        let reply = self.next();
        Box::pin(async move { Ok(reply) })
    }
    fn chat_stream_incremental<'a>(
        &'a self,
        _request: ChatRequest<'a>,
    ) -> Pin<
        Box<dyn std::future::Future<Output = tokio::sync::mpsc::Receiver<StreamEvent>> + Send + 'a>,
    > {
        let reply = self.next();
        Box::pin(async move {
            let (tx, rx) = tokio::sync::mpsc::channel(4);
            let _ = tx.send(StreamEvent::Done(reply)).await;
            rx
        })
    }
}

/// A background tool: it starts the work and asks the model to end its turn.
struct Background;

impl Tool for Background {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "background".to_string().into(),
            description: "starts a job".to_string().into(),
            parameters_schema: r#"{"type":"object"}"#.to_string().into(),
        }
    }
    fn execute(
        &self,
        _arguments: &str,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<ToolResult, DomainError>> + Send + '_>>
    {
        Box::pin(async {
            Ok(ToolResult {
                content: "started — end your turn now".into(),
                is_error: false,
                image_blocks: vec![],
                delivery_metadata: None,
            })
        })
    }
}

fn empty_reply(stop_reason: StopReason) -> LlmResponse {
    LlmResponse {
        content: None,
        tool_calls: vec![],
        usage: None,
        stop_reason: Some(stop_reason),
        thinking_blocks: vec![],
    }
}

fn background_call() -> LlmResponse {
    LlmResponse {
        tool_calls: vec![ToolCall {
            id: "call_background".into(),
            name: "background".into(),
            arguments: "{}".into(),
        }],
        ..empty_reply(StopReason::ToolUse)
    }
}

fn reasoning_only_at_the_limit() -> LlmResponse {
    LlmResponse {
        thinking_blocks: vec![ThinkingBlock::Normal {
            thinking: "a long thought".into(),
            signature: String::new(),
        }],
        ..empty_reply(StopReason::MaxTokens)
    }
}

/// One streamed prompt run: its outcome, its events and the requests made.
async fn run(replies: Vec<LlmResponse>) -> (PromptOutcome, Vec<serde_json::Value>, usize) {
    run_limited(replies, u32::MAX).await
}

/// [`run`] with a tool iteration limit.
async fn run_limited(
    replies: Vec<LlmResponse>,
    max_tool_iterations: u32,
) -> (PromptOutcome, Vec<serde_json::Value>, usize) {
    let provider = Arc::new(ScriptedStream {
        replies: Mutex::new(replies),
        requests: Mutex::new(0),
    });
    let mut registry = crate::infrastructure::tools::registry::ToolRegistryImpl::new();
    registry.register(Arc::new(Background));
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: provider.clone(),
        tool_registry: Box::new(registry),
        model: "stub".into(),
        max_tokens: 100,
        temperature: 0.0,
        retention: None,
        session_key: "cli:test-2434".into(),
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: true,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_marks: Default::default(),
        model_context_window: None,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    })
    .with_max_tool_iterations(max_tool_iterations);
    let mut messages: Vec<Message> = vec![];
    let mut session = AgentSession::new("stub".into());
    let (_cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
    let mut notification_rx = None;
    let subagent_registry = None;
    let mut bytes: Vec<u8> = Vec::new();
    let outcome = {
        let mut sink = EventSink::writer(&mut bytes);
        run_agent_message(PromptRun {
            execution_state: None,
            agent: &mut agent,
            messages: &mut messages,
            active_session: None,
            session: &mut session,
            sink: &mut sink,
            message: Message::user("start the job"),
            system_prompt: "",
            cancel_rx,
            notification_rx: &mut notification_rx,
            subagent_registry: &subagent_registry,
            turn_save: None,
        })
        .await
    };
    let events = String::from_utf8_lossy(&bytes)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("event JSON"))
        .collect();
    let requests = *provider.requests.lock().unwrap();
    (outcome, events, requests)
}

fn agent_end(events: &[serde_json::Value]) -> bool {
    events.iter().any(|event| event["type"] == "agent_end")
}

fn agent_error(events: &[serde_json::Value]) -> Option<String> {
    events
        .iter()
        .find(|event| event["command"] == "agent_error")
        .map(|event| event["error"].as_str().unwrap_or_default().to_string())
}

#[tokio::test]
async fn an_empty_reply_to_tool_results_ends_the_run_with_agent_end() {
    let (outcome, events, requests) =
        run(vec![background_call(), empty_reply(StopReason::EndTurn)]).await;
    assert!(matches!(outcome, PromptOutcome::Success), "{events:?}");
    assert!(agent_end(&events), "{events:?}");
    assert_eq!(agent_error(&events), None, "{events:?}");
    assert_eq!(requests, 2, "the empty reply is never asked again");
}

#[tokio::test]
async fn an_empty_first_reply_to_a_prompt_ends_the_run_with_agent_error() {
    let empty = || empty_reply(StopReason::EndTurn);
    let (outcome, events, requests) = run(vec![empty(), empty(), empty()]).await;
    assert!(matches!(outcome, PromptOutcome::Error), "{events:?}");
    assert!(!agent_end(&events), "{events:?}");
    let error = agent_error(&events).expect("agent_error");
    assert!(error.contains("synthetic=empty_stream"), "{error}");
    assert_eq!(requests, 3, "retried as today");
}

#[tokio::test]
async fn a_reasoning_only_reply_at_the_limit_after_tools_ends_the_run_with_agent_error() {
    let (outcome, events, requests) = run(vec![
        background_call(),
        reasoning_only_at_the_limit(),
        reasoning_only_at_the_limit(),
    ])
    .await;
    assert!(matches!(outcome, PromptOutcome::Error), "{events:?}");
    assert!(!agent_end(&events), "{events:?}");
    let error = agent_error(&events).expect("agent_error");
    assert!(error.contains("stop_reason=max_tokens"), "{error}");
    assert_eq!(requests, 3, "asked again once, as #2124 does");
}

/// The byte length of the run's reply text, as `turn_end` and `agent_end`
/// report it.
fn content_lengths(events: &[serde_json::Value]) -> (Option<u64>, Option<u64>) {
    let of = |kind: &str| events.iter().find(|event| event["type"] == kind);
    (
        of("turn_end").and_then(|event| event["message"]["contentLength"].as_u64()),
        of("agent_end").and_then(|event| event["contentLength"].as_u64()),
    )
}

/// Review round 1 (L1): a run that recorded no reply reports a reply of no
/// text on both end events, so a client never rebuilds a reply that does
/// not exist.
#[tokio::test]
async fn a_run_ended_by_an_empty_reply_reports_no_reply_text() {
    let (_, events, _) = run(vec![background_call(), empty_reply(StopReason::EndTurn)]).await;
    assert_eq!(content_lengths(&events), (Some(0), Some(0)), "{events:?}");
}

/// The tool iteration limit records no reply either: its notice is the
/// run's response, not a message the run recorded.
#[tokio::test]
async fn a_run_stopped_at_the_tool_iteration_limit_reports_no_reply_text() {
    let (outcome, events, requests) = run_limited(vec![background_call()], 1).await;
    assert!(matches!(outcome, PromptOutcome::Success), "{events:?}");
    assert_eq!(requests, 1);
    assert_eq!(content_lengths(&events), (Some(0), Some(0)), "{events:?}");
}

#[tokio::test]
async fn a_run_with_a_reply_reports_its_text_length_on_both_end_events() {
    let reply = LlmResponse {
        content: Some("done".into()),
        ..empty_reply(StopReason::EndTurn)
    };
    let (_, events, _) = run(vec![background_call(), reply]).await;
    assert_eq!(content_lengths(&events), (Some(4), Some(4)), "{events:?}");
}
