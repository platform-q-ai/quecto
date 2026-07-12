//! Regression tests for #994/#1022: unify the duplicated UDS streaming pipeline
//! and keep the follow-up invariants pinned.
//!
//! The behaviours exercised here are the public UDS wire contracts: parse errors
//! are equivalent across command loops, Writer and Broadcast sinks emit the same
//! event stream, and message snapshots keep the canonical response envelope and
//! public message shape.

use std::sync::Arc;
use std::sync::atomic::AtomicU32;

use super::{ClientCommand, ClientMessage, DispatchCtx, handle_client_msg};
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::domain::message::Message;
use crate::infrastructure::persistence::session_store::FileSessionStore;
use crate::interface::cli::uds_cancel::{CancelHandle, CancelSlot};
use crate::interface::cli::uds_ext_protocol::new_client_tool_registry;
use crate::interface::cli::uds_session::{AgentSession, compute_session_stats};

fn make_agent() -> AgentLoopImpl {
    AgentLoopImpl::new(AgentLoopConfig {
        provider: crate::interface::test_support::make_stub_provider(),
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
    })
}

/// Owns everything a broadcast-wired `DispatchCtx` borrows.
struct Fixture {
    agent: AgentLoopImpl,
    messages: Vec<Message>,
    session: AgentSession,
    session_key: String,
    store: FileSessionStore,
    _tmp: tempfile::TempDir,
    writer: tokio::io::Sink,
    cancel: CancelHandle,
    broadcast_tx: tokio::sync::broadcast::Sender<String>,
}

impl Fixture {
    fn new() -> (Self, tokio::sync::broadcast::Receiver<String>) {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = FileSessionStore::new(tmp.path());
        let (tx, rx) = tokio::sync::broadcast::channel::<String>(64);
        (
            Self {
                agent: make_agent(),
                messages: Vec::new(),
                session: AgentSession::new("stub".into(), "cli:test".into()),
                session_key: "cli:test".to_string(),
                store,
                _tmp: tmp,
                writer: tokio::io::sink(),
                cancel: Arc::new(std::sync::Mutex::new(CancelSlot::Idle)),
                broadcast_tx: tx,
            },
            rx,
        )
    }

    fn ctx(&mut self) -> DispatchCtx<'_> {
        let initial_stats = compute_session_stats(&self.session_key, &self.messages);
        DispatchCtx {
            wire_mode: crate::interface::cli::uds_wire::ConnectionWireMode::legacy(),
            base_dir: self._tmp.path(),
            agent: &mut self.agent,
            messages: &mut self.messages,
            conversation_snapshot: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            state_snapshot: Arc::new(tokio::sync::RwLock::new(
                self.session.state_snapshot(0, None, 0, None),
            )),
            session_stats_snapshot: Arc::new(tokio::sync::RwLock::new(initial_stats)),
            extension_snapshot: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            busy: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            session: &mut self.session,
            stdout: Some(&mut self.writer),
            session_key: &mut self.session_key,
            session_store: &self.store,
            ephemeral: false,
            system_prompt: "",
            cancel_handle: self.cancel.clone(),
            turn_control: Arc::default(),
            broadcast_tx: Some(self.broadcast_tx.clone()),
            ext_registry: None,
            client_tool_registry: new_client_tool_registry(),
            current_client_id: 0,
            subagent_registry: None,
            notification_rx: None,
            workflow_state: None,
            workflow_config: None,
            provider_reload: None,
            provider_reload_inputs: None,
            last_persisted_message_index: 0,
            durable_prefix_dirty: false,
        }
    }
}

async fn multi_client_parse_error_text(line: &str) -> String {
    let (mut fx, mut rx) = Fixture::new();
    let live = AtomicU32::new(1);
    {
        let mut ctx = fx.ctx();
        let msg = ClientMessage::Command(ClientCommand {
            line: line.to_string(),
            client_id: 0,
        });
        let exit = handle_client_msg(&mut ctx, msg, /* persist */ true, &live).await;
        assert!(!exit, "a parse error must not terminate the dispatch loop");
    }
    let line = rx
        .try_recv()
        .expect("a parse_error event should be broadcast");
    let ev: serde_json::Value = serde_json::from_str(line.trim()).expect("valid JSON event");
    assert_eq!(ev["type"], "response");
    assert_eq!(ev["command"], "parse_error");
    assert_eq!(ev["success"], false);
    ev["error"].as_str().unwrap_or("").to_string()
}

fn single_client_parse_error_text(line: &str) -> String {
    // The single-client command loop delegates parse failures to `uds::parse_line`;
    // the BDD coverage exercises both real loops end-to-end.
    match crate::interface::cli::uds::parse_line(line) {
        crate::interface::cli::uds::LineResult::ParseError(err) => err,
        crate::interface::cli::uds::LineResult::Command(_) => {
            panic!("malformed input unexpectedly parsed as a command")
        }
    }
}

#[tokio::test]
async fn command_loops_report_identical_malformed_command_errors() {
    let malformed = "{not valid json";

    let single = single_client_parse_error_text(malformed);
    let multi = multi_client_parse_error_text(malformed).await;

    assert_eq!(multi, single);
    assert!(
        multi.contains("parse error:"),
        "parse error must preserve detailed serde text, got: {multi:?}"
    );
    assert_ne!(multi, "invalid JSON command");
}

/// Golden-JSON test for criterion 3 (#994): the borrowed `MessageView`
/// serializer must produce the exact wire shape the old `json!` tree did, for
/// every role and for fully-populated tool-call fields.
#[test]
fn message_to_json_matches_golden_wire_shape_for_all_roles() {
    use crate::domain::message::ToolCall;
    use crate::interface::cli::uds_session::message_to_json;

    let assistant = Message::assistant(
        "calling a tool",
        vec![ToolCall {
            id: "tc-1".to_string(),
            name: "read".to_string(),
            arguments: "{\"path\":\"x\"}".to_string(),
        }],
    );
    let assistant_json = message_to_json(&assistant);
    assert_eq!(assistant_json["role"], "assistant");
    assert_eq!(assistant_json["content"], "calling a tool");
    assert_eq!(
        assistant_json["toolCalls"],
        serde_json::json!([{"id": "tc-1", "name": "read", "arguments": "{\"path\":\"x\"}"}])
    );
    assert!(assistant_json["toolCallId"].is_null());
    assert!(assistant_json["toolName"].is_null());
    assert_eq!(assistant_json["id"], assistant.id().to_string());

    let mut tool = Message::tool("tc-1", "result body");
    tool.tool_name = Some("read".to_string());
    let tool_json = message_to_json(&tool);
    assert_eq!(tool_json["role"], "tool");
    assert_eq!(tool_json["content"], "result body");
    assert_eq!(tool_json["toolCalls"], serde_json::json!([]));
    assert_eq!(tool_json["toolCallId"], "tc-1");
    assert_eq!(tool_json["toolName"], "read");
    assert_eq!(tool_json["id"], tool.id().to_string());

    for (msg, want) in [
        (Message::system("s"), "system"),
        (Message::user("u"), "user"),
        (Message::assistant("a", vec![]), "assistant"),
        (Message::tool("id", "t"), "tool"),
    ] {
        assert_eq!(message_to_json(&msg)["role"], want, "role {:?}", msg.role);
    }
}

/// Envelope-equivalence test for criterion 4 (#994): the hand-rolled
/// `GetMessagesSnapshot` serializer in `build_get_messages_line` must stay
/// value-identical (modulo key order) to the canonical
/// `AgentEvent::ok(None, "get_messages", …)` envelope, so the two cannot
/// silently drift if `AgentEvent::Response` changes.
#[test]
fn get_messages_snapshot_line_matches_agent_event_envelope() {
    use crate::domain::message::ToolCall;
    use crate::interface::cli::protocol::AgentEvent;
    use crate::interface::cli::uds_session::message_to_json;
    use crate::interface::cli::uds_snapshots::build_get_messages_line;

    let messages = vec![
        Message::user("hello"),
        Message::assistant(
            "hi",
            vec![ToolCall {
                id: "tc-9".to_string(),
                name: "bash".to_string(),
                arguments: "{}".to_string(),
            }],
        ),
        Message::tool("tc-9", "output"),
    ];

    let line = build_get_messages_line(&messages);
    let got: serde_json::Value = serde_json::from_str(line.trim()).expect("snapshot line is JSON");
    let assistant = got["data"]["messages"]
        .as_array()
        .expect("messages array")
        .iter()
        .find(|message| message["role"] == "assistant")
        .expect("assistant message");
    let tool_calls = assistant["toolCalls"].as_array().expect("toolCalls array");
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0]["id"], "tc-9");
    assert_eq!(tool_calls[0]["name"], "bash");

    let msgs_json: Vec<serde_json::Value> = messages.iter().map(message_to_json).collect();
    let canonical = AgentEvent::ok(
        None,
        "get_messages",
        Some(serde_json::json!({ "messages": msgs_json, "snapshot": true })),
    );
    let want: serde_json::Value =
        serde_json::from_str(&canonical.to_json_line()).expect("canonical event is JSON");

    assert_eq!(got, want);
}

#[test]
fn trimmed_get_messages_snapshot_line_matches_agent_event_envelope() {
    use crate::interface::cli::protocol::AgentEvent;
    use crate::interface::cli::uds_session::message_to_json;
    use crate::interface::cli::uds_snapshots::build_get_messages_line;

    let messages = vec![Message::user("x".repeat(2 * 1024 * 1024))];

    let line = build_get_messages_line(&messages);
    let got: serde_json::Value = serde_json::from_str(line.trim()).expect("snapshot line is JSON");

    let canonical = AgentEvent::ok(
        None,
        "get_messages",
        Some(serde_json::json!({
            "messages": Vec::<serde_json::Value>::new(),
            "snapshot": true,
            "trimmed": true,
        })),
    );
    let want: serde_json::Value =
        serde_json::from_str(&canonical.to_json_line()).expect("canonical event is JSON");

    assert_eq!(got, want);
    assert_eq!(got["data"]["snapshot"], true);
    assert_eq!(got["data"]["messages"].as_array().unwrap().len(), 0);
    assert_eq!(got["data"]["trimmed"], true);
    assert!(line.len() <= 1024 * 1024);

    let would_not_trim = AgentEvent::ok(
        None,
        "get_messages",
        Some(serde_json::json!({
            "messages": messages.iter().map(message_to_json).collect::<Vec<_>>(),
            "snapshot": true,
        })),
    );
    let untrimmed: serde_json::Value = serde_json::from_str(&would_not_trim.to_json_line())
        .expect("canonical untrimmed event is JSON");
    assert_ne!(
        got, untrimmed,
        "oversized snapshots must be visibly trimmed"
    );
}

#[test]
fn under_budget_get_messages_snapshot_stays_untrimmed() {
    use crate::interface::cli::uds_snapshots::{
        SNAPSHOT_MESSAGES_BUDGET_BYTES, build_get_messages_line,
    };

    let content = "x".repeat(SNAPSHOT_MESSAGES_BUDGET_BYTES / 2);
    let messages = vec![Message::user(content.clone())];
    let line = build_get_messages_line(&messages);
    let got: serde_json::Value = serde_json::from_str(line.trim()).expect("snapshot line is JSON");

    assert!(line.len() <= 1024 * 1024);
    assert_ne!(
        got["data"]["trimmed"], true,
        "a half-budget message must not be trimmed"
    );
    assert_eq!(got["data"]["messages"].as_array().unwrap().len(), 1);
    assert_eq!(got["data"]["messages"][0]["content"], content);
}

#[tokio::test]
async fn event_sink_variants_emit_identical_json_for_same_events() {
    use crate::interface::cli::protocol::{AgentEvent, ToolResultContent, TurnMessage, TurnUsage};
    use crate::interface::cli::uds_cancel::EventSink;

    let events = vec![
        AgentEvent::AgentStart,
        AgentEvent::TurnStart,
        AgentEvent::Token {
            token: "hello".to_string(),
        },
        AgentEvent::TurnEnd {
            message: TurnMessage {
                role: "assistant".to_string(),
                content: "done".to_string(),
                message_refs: vec![],
                usage: Some(TurnUsage {
                    input: 1,
                    output: 2,
                    total: 3,
                }),
                stop_reason: None,
                context_tokens: Some(4),
                max_context_tokens: Some(100),
            },
            tool_results: vec![],
        },
        AgentEvent::ToolExecutionEnd {
            tool_call_id: "tc-1".to_string(),
            tool_name: "read".to_string(),
            result: ToolResultContent {
                content: vec![serde_json::json!({"type":"text","text":"ok"})],
            },
            is_error: false,
        },
        AgentEvent::AgentEnd {
            messages: vec![],
            message_refs: vec![],
        },
    ];

    let mut writer_bytes = Vec::new();
    {
        let mut sink = EventSink::writer(&mut writer_bytes);
        for event in &events {
            sink.emit(event).await;
        }
    }
    let writer_lines: Vec<String> = String::from_utf8(writer_bytes)
        .expect("writer output is UTF-8")
        .lines()
        .map(str::to_string)
        .collect();

    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(events.len() + 1);
    let mut sink = EventSink::Broadcast(tx);
    for event in &events {
        sink.emit(event).await;
    }
    let mut broadcast_lines = Vec::new();
    while broadcast_lines.len() < events.len() {
        broadcast_lines.push(
            rx.try_recv()
                .expect("broadcast event")
                .trim_end()
                .to_string(),
        );
    }

    assert_eq!(broadcast_lines, writer_lines);
}

async fn run_stub_turn_event_types_with_sink(
    sink: &mut crate::interface::cli::uds_cancel::EventSink<'_>,
) -> Vec<String> {
    use crate::interface::cli::uds_cancel::{PromptRun, run_agent_message};

    let mut agent = make_agent();
    let mut messages = Vec::new();
    let mut session = AgentSession::new("stub".into(), "cli:test".into());
    let (_cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
    let mut notification_rx = None;
    let subagent_registry = None;

    let outcome = run_agent_message(PromptRun {
        agent: &mut agent,
        messages: &mut messages,
        session: &mut session,
        sink,
        message: Message::user("hello"),
        cancel_rx,
        notification_rx: &mut notification_rx,
        subagent_registry: &subagent_registry,
    })
    .await;
    assert!(matches!(
        outcome,
        crate::interface::cli::uds_cancel::PromptOutcome::Success
    ));

    messages
        .iter()
        .map(|message| format!("{:?}", message.role))
        .collect()
}

#[tokio::test]
async fn stub_provider_turn_has_same_event_sequence_through_each_sink() {
    let mut writer_bytes = Vec::new();
    let writer_message_roles = {
        let mut sink = crate::interface::cli::uds_cancel::EventSink::writer(&mut writer_bytes);
        run_stub_turn_event_types_with_sink(&mut sink).await
    };
    let writer_event_types: Vec<String> = String::from_utf8(writer_bytes)
        .expect("writer output is UTF-8")
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("event JSON"))
        .map(|event| event["type"].as_str().unwrap_or_default().to_string())
        .collect();

    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(16);
    let broadcast_message_roles = {
        let mut sink = crate::interface::cli::uds_cancel::EventSink::Broadcast(tx);
        run_stub_turn_event_types_with_sink(&mut sink).await
    };
    let mut broadcast_event_types = Vec::new();
    while broadcast_event_types.len() < writer_event_types.len() {
        let line = rx.try_recv().expect("broadcast event");
        let event: serde_json::Value = serde_json::from_str(line.trim()).expect("event JSON");
        broadcast_event_types.push(event["type"].as_str().unwrap_or_default().to_string());
    }

    assert_eq!(broadcast_event_types, writer_event_types);
    assert_eq!(broadcast_message_roles, writer_message_roles);
    // #1060: non-streaming stub turns surface the assistant text once as a
    // Token before the ref-based turn_end / agent_end (no content re-carry).
    assert_eq!(
        writer_event_types,
        vec![
            "agent_start",
            "turn_start",
            "token",
            "turn_end",
            "agent_end"
        ]
    );
}
