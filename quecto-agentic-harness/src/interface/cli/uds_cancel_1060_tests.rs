//! #1060 — production emit path for ref-based end-of-turn events.
//!
//! Drives `run_agent_message` with a stub agent (same pattern as #1072 e2e)
//! and asserts the live `turn_end` / `agent_end` events carry non-empty
//! message refs, do not re-carry full content, and stay small for a real
//! large-turn body. These must FAIL before the emit-path change lands (RED).

use std::pin::Pin;
use std::sync::{Arc, Mutex};

use super::{EventSink, PromptOutcome, PromptRun, run_agent_message};
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::domain::error::DomainError;
use crate::domain::message::{LlmResponse, Message, ToolCall};
use crate::domain::provider::{ChatRequest, LlmProvider, StreamEvent};
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use crate::interface::cli::protocol::EVENT_LINE_CAP_BYTES;
use crate::interface::cli::uds_session::AgentSession;

#[derive(Debug)]
struct ScriptedProvider {
    responses: Mutex<Vec<LlmResponse>>,
}

impl LlmProvider for ScriptedProvider {
    fn name(&self) -> &str {
        "scripted-1060"
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

/// Emits each delta as a `TextDelta` stream event (which the agent forwards as
/// an `AgentProgressEvent::Token`, so `tokens_emitted == true`) before the
/// assembled `Done` response. Used to prove the producer does NOT append a
/// duplicate synthetic token on a streaming turn (#1060).
#[derive(Debug)]
struct StreamingProvider {
    deltas: Vec<String>,
    response: LlmResponse,
}

impl LlmProvider for StreamingProvider {
    fn name(&self) -> &str {
        "streaming-1060"
    }

    fn chat(
        &self,
        _request: ChatRequest<'_>,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<LlmResponse, DomainError>> + Send + '_>>
    {
        let resp = self.response.clone();
        Box::pin(async move { Ok(resp) })
    }

    fn chat_stream_incremental<'a>(
        &'a self,
        _request: ChatRequest<'a>,
    ) -> Pin<
        Box<dyn std::future::Future<Output = tokio::sync::mpsc::Receiver<StreamEvent>> + Send + 'a>,
    > {
        let deltas = self.deltas.clone();
        let resp = self.response.clone();
        Box::pin(async move {
            let (tx, rx) = tokio::sync::mpsc::channel(32);
            for d in deltas {
                let _ = tx.send(StreamEvent::TextDelta(d)).await;
            }
            let _ = tx.send(StreamEvent::Done(resp)).await;
            rx
        })
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

fn text_response(content: &str) -> LlmResponse {
    LlmResponse {
        content: Some(content.to_string()),
        tool_calls: vec![],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
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

fn event_lines(bytes: &[u8]) -> Vec<serde_json::Value> {
    String::from_utf8_lossy(bytes)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("event JSON"))
        .collect()
}

fn non_empty_refs(v: &serde_json::Value) -> Vec<String> {
    let candidates = [
        v.get("messageRefs"),
        v.get("message").and_then(|m| m.get("messageRefs")),
        v.get("message_refs"),
        v.get("message").and_then(|m| m.get("message_refs")),
    ];
    for c in candidates.into_iter().flatten() {
        if let Some(arr) = c.as_array() {
            let refs: Vec<String> = arr
                .iter()
                .filter_map(|item| {
                    if let Some(s) = item.as_str() {
                        return Some(s.to_string());
                    }
                    item.get("id")
                        .and_then(|id| id.as_str())
                        .map(str::to_string)
                })
                .filter(|s| !s.is_empty())
                .collect();
            if !refs.is_empty() {
                return refs;
            }
        }
    }
    Vec::new()
}

async fn run_turn(
    responses: Vec<LlmResponse>,
    tools: Vec<(ToolDefinition, String)>,
    prompt: &str,
) -> (Vec<Message>, Vec<u8>) {
    let mut registry = crate::infrastructure::tools::registry::ToolRegistryImpl::new();
    for (def, output) in tools {
        registry.register(Arc::new(FixedTool { def, output }));
    }
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: Arc::new(ScriptedProvider {
            responses: Mutex::new(responses),
        }),
        tool_registry: Box::new(registry),
        model: "stub".into(),
        max_tokens: 100,
        temperature: 0.0,
        spill_store: None,
        session_key: "cli:test-1060".into(),
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
    let mut messages: Vec<Message> = vec![];
    let mut session = AgentSession::new("stub".into(), "cli:test-1060".into());
    let (_cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
    let mut notification_rx = None;
    let subagent_registry = None;
    let mut writer_bytes: Vec<u8> = Vec::new();
    let outcome = {
        let mut sink = EventSink::writer(&mut writer_bytes);
        run_agent_message(PromptRun {
            agent: &mut agent,
            messages: &mut messages,
            conversation_snapshot: None,
            session: &mut session,
            sink: &mut sink,
            message: Message::user(prompt),
            cancel_rx,
            notification_rx: &mut notification_rx,
            subagent_registry: &subagent_registry,
        })
        .await
    };
    assert!(
        matches!(outcome, PromptOutcome::Success),
        "turn must complete"
    );
    (messages, writer_bytes)
}

/// Drive one streaming turn: the provider emits `deltas` as token progress
/// events (`tokens_emitted == true`) then an assembled response.
async fn run_streaming_turn(deltas: Vec<&str>, response: &str, prompt: &str) -> Vec<u8> {
    let provider = Arc::new(StreamingProvider {
        deltas: deltas.iter().map(|d| d.to_string()).collect(),
        response: text_response(response),
    });
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(crate::infrastructure::tools::registry::ToolRegistryImpl::new()),
        model: "stub".into(),
        max_tokens: 100,
        temperature: 0.0,
        spill_store: None,
        session_key: "cli:test-1060".into(),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: true,
        effort: None,
        system_prompt_provider: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
    });
    let mut messages: Vec<Message> = vec![];
    let mut session = AgentSession::new("stub".into(), "cli:test-1060".into());
    let (_cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
    let mut notification_rx = None;
    let subagent_registry = None;
    let mut writer_bytes: Vec<u8> = Vec::new();
    let outcome = {
        let mut sink = EventSink::writer(&mut writer_bytes);
        run_agent_message(PromptRun {
            agent: &mut agent,
            messages: &mut messages,
            conversation_snapshot: None,
            session: &mut session,
            sink: &mut sink,
            message: Message::user(prompt),
            cancel_rx,
            notification_rx: &mut notification_rx,
            subagent_registry: &subagent_registry,
        })
        .await
    };
    assert!(
        matches!(outcome, PromptOutcome::Success),
        "turn must complete"
    );
    writer_bytes
}

/// #1060: on a STREAMING turn (`tokens_emitted == true`) the producer must NOT
/// append a synthetic full-text token — the deltas already carried the content.
/// A duplicate would re-ship the whole response the ref-based end-of-turn is
/// meant to avoid re-carrying.
#[tokio::test]
async fn streaming_turn_emits_no_duplicate_synthetic_token() {
    let deltas = vec!["Hello ", "streamed ", "world"];
    let full = "Hello streamed world";
    let bytes = run_streaming_turn(deltas.clone(), full, "say hi").await;
    let events = event_lines(&bytes);

    let token_texts: Vec<String> = events
        .iter()
        .filter(|e| e["type"] == "token")
        .map(|e| e["token"].as_str().unwrap_or("").to_string())
        .collect();

    // Exactly the streamed deltas — no extra synthetic token carrying `full`.
    assert_eq!(
        token_texts, deltas,
        "streaming turn must emit only the streamed deltas, with no synthetic \
         duplicate token; got {token_texts:?}"
    );
    assert!(
        !token_texts.iter().any(|t| t == full),
        "no single token event may carry the whole assembled response (that is \
         the synthetic-token path, which must fire only for non-streaming turns)"
    );

    // End-of-turn still ref-based (content not re-carried).
    let turn_end = events
        .iter()
        .find(|e| e["type"] == "turn_end")
        .expect("turn_end emitted");
    assert!(
        !non_empty_refs(turn_end).is_empty(),
        "turn_end must carry refs"
    );
    assert_eq!(
        turn_end["message"]["content"].as_str(),
        Some(""),
        "streaming turn_end must still empty its content: {turn_end}"
    );
}

/// #1060 AC1/AC6: production turn_end / agent_end carry non-empty refs and
/// do not re-carry the full assistant body for a real large response.
#[tokio::test]
async fn production_large_turn_end_of_turn_events_use_refs_not_full_content() {
    // Real non-empty content — larger than a quarter of the frame cap so a
    // full re-carry would dominate the event size.
    let body = format!(
        "REAL-LARGE-ASSISTANT-RESPONSE-{}",
        "Z".repeat(EVENT_LINE_CAP_BYTES / 2)
    );
    let (messages, bytes) =
        run_turn(vec![text_response(&body)], vec![], "write a long answer").await;

    let events = event_lines(&bytes);
    let turn_end = events
        .iter()
        .find(|e| e["type"] == "turn_end")
        .expect("turn_end must be emitted");
    let agent_end = events
        .iter()
        .find(|e| e["type"] == "agent_end")
        .expect("agent_end must be emitted");

    let turn_refs = non_empty_refs(turn_end);
    let agent_refs = non_empty_refs(agent_end);
    assert!(
        !turn_refs.is_empty(),
        "production turn_end must carry non-empty messageRefs (#1060); got: {turn_end}"
    );
    assert!(
        !agent_refs.is_empty(),
        "production agent_end must carry non-empty messageRefs (#1060); got: {agent_end}"
    );

    // Refs must round-trip domain ids of the messages this run produced.
    let domain_ids: Vec<String> = messages
        .iter()
        .filter(|m| m.role != crate::domain::message::Role::User)
        .map(|m| m.id().to_string())
        .collect();
    for id in &domain_ids {
        assert!(
            agent_refs.iter().any(|r| r == id),
            "agent_end refs must include domain message id {id}; refs={agent_refs:?}"
        );
    }

    let content = turn_end["message"]["content"].as_str().unwrap_or("");
    assert!(
        content.is_empty() || !content.contains("REAL-LARGE-ASSISTANT-RESPONSE"),
        "production turn_end must not re-carry the large assistant body"
    );
    if let Some(msgs) = agent_end.get("messages").and_then(|m| m.as_array()) {
        for m in msgs {
            let c = m.get("content").and_then(|c| c.as_str()).unwrap_or("");
            assert!(
                c.is_empty() || !c.contains("REAL-LARGE-ASSISTANT-RESPONSE"),
                "production agent_end must not re-carry full content"
            );
        }
    }

    let turn_line = events
        .iter()
        .find(|e| e["type"] == "turn_end")
        .map(|e| e.to_string())
        .unwrap();
    let agent_line = events
        .iter()
        .find(|e| e["type"] == "agent_end")
        .map(|e| e.to_string())
        .unwrap();
    let budget = EVENT_LINE_CAP_BYTES / 4;
    assert!(
        turn_line.len() < budget,
        "large-turn turn_end must stay well under the frame cap; got {}",
        turn_line.len()
    );
    assert!(
        agent_line.len() < budget,
        "large-turn agent_end must stay well under the frame cap; got {}",
        agent_line.len()
    );

    // Footer metadata still present.
    assert!(
        turn_end["message"]["contextTokens"].is_number()
            || turn_end["message"]["maxContextTokens"].is_number(),
        "turn_end must retain footer context metadata: {turn_end}"
    );
}

/// #1060 AC6: tool-using turn refs cover assistant tool-call + tool-result roles.
#[tokio::test]
async fn production_tool_turn_agent_end_refs_cover_all_roles() {
    // Real large tool arguments/results must flow through production. If the
    // producer ever re-carries either body, the emitted-size assertions below
    // fail rather than merely measuring a hand-constructed empty event.
    let large_args = "A".repeat(EVENT_LINE_CAP_BYTES + 4096);
    let large_result = "R".repeat(EVENT_LINE_CAP_BYTES + 4096);
    let mut tool_response = tool_call_response("bulk");
    tool_response.tool_calls[0].arguments =
        serde_json::json!({ "payload": large_args }).to_string();
    let (messages, bytes) = run_turn(
        vec![tool_response, text_response("final answer body")],
        vec![(
            ToolDefinition {
                name: "bulk".to_string().into(),
                description: "bulk".to_string().into(),
                parameters_schema: r#"{"type":"object"}"#.to_string().into(),
            },
            large_result.clone(),
        )],
        "run a tool",
    )
    .await;

    let events = event_lines(&bytes);
    let agent_end = events
        .iter()
        .find(|e| e["type"] == "agent_end")
        .expect("agent_end");
    let refs = non_empty_refs(agent_end);
    assert!(
        serde_json::to_vec(agent_end).unwrap().len() < EVENT_LINE_CAP_BYTES / 4,
        "production agent_end must stay bounded despite large real tool args/results"
    );
    assert!(
        refs.len() >= 3,
        "tool turn agent_end must ref assistant tool-call, tool result, and final text \
         (got {} refs): {agent_end}",
        refs.len()
    );
    let domain_ids: Vec<String> = messages
        .iter()
        .filter(|m| m.role != crate::domain::message::Role::User)
        .map(|m| m.id().to_string())
        .collect();
    assert_eq!(
        domain_ids.len(),
        3,
        "scenario: run must append assistant+tool+assistant"
    );
    for id in &domain_ids {
        assert!(
            refs.iter().any(|r| r == id),
            "missing domain id {id} in agent_end refs {refs:?}"
        );
    }
    // No full content re-carry.
    if let Some(msgs) = agent_end.get("messages").and_then(|m| m.as_array()) {
        for m in msgs {
            let c = m.get("content").and_then(|c| c.as_str()).unwrap_or("");
            assert!(
                c.is_empty() || (c != "final answer body" && c != large_result),
                "agent_end must not re-carry full tool-turn content"
            );
        }
    }
}
