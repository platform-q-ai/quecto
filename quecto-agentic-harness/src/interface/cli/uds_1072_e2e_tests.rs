//! #1072 end-to-end regression: `run_agent_message` driven with a stub agent
//! whose turn shrinks history below its pre-turn length mid-run — the live
//! panic shape (`range start index 180 out of range for slice of length 80`).
//!
//! The pre-fix code sliced `messages[before_len..]` for the AgentEnd payload
//! and truncated at a pre-turn `user_msg_idx` on cancel; both panic or emit
//! wrong data once pruning removes earlier vector entries. This test fails
//! (by panic or by payload mismatch) if those call sites ever revert to
//! positional slicing, and fails if the durable-prefix dirty latch is
//! dropped from the pruning pass.

use std::pin::Pin;
use std::sync::{Arc, Mutex};

use super::{EventSink, PromptOutcome, PromptRun, run_agent_message};
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::domain::error::DomainError;
use crate::domain::message::{LlmResponse, Message, ToolCall};
use crate::domain::provider::{ChatRequest, LlmProvider};
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use crate::interface::cli::uds_session::AgentSession;

/// Provider returning a scripted FIFO of responses.
#[derive(Debug)]
struct ScriptedProvider {
    responses: Mutex<Vec<LlmResponse>>,
}

impl LlmProvider for ScriptedProvider {
    fn name(&self) -> &str {
        "scripted"
    }

    fn chat(
        &self,
        _request: ChatRequest<'_>,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<LlmResponse, DomainError>> + Send + '_>>
    {
        let response = self.responses.lock().unwrap().remove(0);
        Box::pin(async move { Ok(response) })
    }
}

struct FixedTool {
    def: ToolDefinition,
    output: String,
}

impl Tool for FixedTool {
    fn definition(&self) -> ToolDefinition {
        self.def.clone()
    }

    fn execute(
        &self,
        _arguments: &str,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<ToolResult, DomainError>> + Send + '_>>
    {
        let output = self.output.clone();
        Box::pin(async move {
            Ok(ToolResult {
                content: output,
                is_error: false,
                image_blocks: vec![],
            })
        })
    }
}

fn tool_call_response(name: &str) -> LlmResponse {
    LlmResponse {
        content: None,
        tool_calls: vec![ToolCall {
            id: format!("call_{name}"),
            name: name.to_string(),
            arguments: "{}".to_string(),
        }],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    }
}

fn text_response(content: &str) -> LlmResponse {
    LlmResponse {
        content: Some(content.to_string()),
        tool_calls: vec![],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    }
}

/// A big assistant message from an earlier prompt, droppable by ladder rung 2
/// (no spill_id: rung 1 skips it, rung 2 drops it — pure physical shrink).
fn droppable_history_message(turn: u32) -> Message {
    let mut msg = Message::assistant("lorem ipsum dolor sit amet ".repeat(90), vec![]);
    msg.turn = Some(turn);
    msg
}

/// Drives the full production prompt pipeline (`run_agent_message`) with a
/// stub agent whose mid-run pruning shrinks the conversation BELOW its
/// pre-turn length, then asserts:
///   1. the run completes (the pre-fix positional slice aborts here);
///   2. AgentEnd carries exactly the messages this run appended;
///   3. the agent latch reports the durable prefix dirty, so persistence
///      reconciles.
#[tokio::test]
async fn shrinking_turn_emits_exactly_the_run_appended_messages_and_dirty_flag() {
    let mut registry = crate::infrastructure::tools::registry::ToolRegistryImpl::new();
    registry.register(Arc::new(FixedTool {
        def: ToolDefinition {
            name: "bulk".to_string().into(),
            description: "bulk".to_string().into(),
            parameters_schema: r#"{"type":"object"}"#.to_string().into(),
        },
        output: "tool-output-payload".to_string(),
    }));
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: Arc::new(ScriptedProvider {
            responses: Mutex::new(vec![
                tool_call_response("bulk"),
                text_response("final answer"),
            ]),
        }),
        tool_registry: Box::new(registry),
        model: "stub".into(),
        max_tokens: 100,
        temperature: 0.0,
        spill_store: None,
        session_key: "cli:test".into(),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens: 700,
        progress_callback: None,
        streaming: false,
        effort: None,
        system_prompt_provider: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
    });

    // 8 oversized prior turns: the 700-token budget forces rung 2 to drop
    // most of them on the first prune, before any provider call.
    let mut messages: Vec<Message> = (1..=8).map(droppable_history_message).collect();
    let pre_turn_len = messages.len() + 1; // history + the incoming prompt

    let mut session = AgentSession::new("stub".into(), "cli:test".into());
    let (_cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
    let mut notification_rx = None;
    let subagent_registry = None;
    let mut writer_bytes: Vec<u8> = Vec::new();
    let outcome = {
        let mut sink = EventSink::writer(&mut writer_bytes);
        run_agent_message(PromptRun {
            agent: &mut agent,
            messages: &mut messages,
            session: &mut session,
            sink: &mut sink,
            message: Message::user("go"),
            cancel_rx,
            notification_rx: &mut notification_rx,
            subagent_registry: &subagent_registry,
        })
        .await
    };

    // The live failure shape really happened: post-turn length is below the
    // pre-turn watermark, so `messages[before_len..]` would have panicked.
    assert!(
        messages.len() < pre_turn_len,
        "scenario setup: pruning must shrink history below its pre-turn \
         length ({pre_turn_len} -> {})",
        messages.len()
    );

    assert!(
        matches!(outcome, PromptOutcome::Success),
        "the shrinking run must still complete successfully"
    );
    assert!(
        agent.take_durable_prefix_dirty(),
        "a run that dropped pre-existing history must latch the durable \
         prefix dirty so persistence reconciles — the latch (drained by \
         persist_current_session) is the single authoritative channel"
    );

    let agent_end = String::from_utf8(writer_bytes)
        .expect("writer output is UTF-8")
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("event JSON"))
        .find(|event| event["type"] == "agent_end")
        .expect("an agent_end event must be emitted");
    // #1060: AgentEnd identifies the run via messageRefs (not full content).
    let refs = agent_end["messageRefs"]
        .as_array()
        .expect("messageRefs array");
    assert_eq!(
        refs.len(),
        3,
        "AgentEnd must ref exactly the messages this run appended          (assistant tool call, tool result, final reply), got {agent_end}"
    );
    assert!(
        refs.iter()
            .all(|r| r.as_str().is_some_and(|s| !s.is_empty()))
    );
    let payload = agent_end["messages"].as_array().expect("messages array");
    assert!(
        payload.is_empty(),
        "AgentEnd must not re-carry full message content after #1060: {payload:?}"
    );
}

/// Clean-side boundary (#1072 review, coverage finding 2), dispatch level: a
/// turn far under budget must leave the agent's dirty latch CLEAR so the
/// UDS layer keeps the append-only `save_clean_delta` fast path. Fails if the
/// latch is hardcoded true or latched on a non-mutating prune pass.
#[tokio::test]
async fn under_budget_turn_reports_prefix_clean_on_its_outcome() {
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: Arc::new(ScriptedProvider {
            responses: Mutex::new(vec![text_response("ok")]),
        }),
        tool_registry: Box::new(crate::infrastructure::tools::registry::ToolRegistryImpl::new()),
        model: "stub".into(),
        max_tokens: 100,
        temperature: 0.0,
        spill_store: None,
        session_key: "cli:test".into(),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        system_prompt_provider: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
    });
    let mut messages = vec![
        Message::user("earlier"),
        Message::assistant("prior", vec![]),
    ];

    let mut session = AgentSession::new("stub".into(), "cli:test".into());
    let (_cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
    let mut notification_rx = None;
    let subagent_registry = None;
    let mut writer_bytes: Vec<u8> = Vec::new();
    let outcome = {
        let mut sink = EventSink::writer(&mut writer_bytes);
        run_agent_message(PromptRun {
            agent: &mut agent,
            messages: &mut messages,
            session: &mut session,
            sink: &mut sink,
            message: Message::user("hi"),
            cancel_rx,
            notification_rx: &mut notification_rx,
            subagent_registry: &subagent_registry,
        })
        .await
    };

    assert!(
        matches!(outcome, PromptOutcome::Success),
        "the under-budget turn must complete successfully"
    );
    assert!(
        !agent.take_durable_prefix_dirty(),
        "an under-budget turn must NOT latch the durable prefix dirty — a \
         spurious dirty latch forces a full compact-rewrite every turn and \
         defeats the save_clean_delta fast path"
    );
}
