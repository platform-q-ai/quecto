//! Regression tests for #994: unify the duplicated UDS streaming pipeline and
//! cut per-message serialization churn.
//!
//! The behavioural acceptance criterion exercised here is #2 — parse-error
//! handling must be *consistent* across the two command loops. The single
//! client loop (`uds::run_command_loop`) surfaces the detailed serde error
//! text via `AgentEvent::err(None, "parse_error", e)`; historically the multi
//! client dispatch loop (`uds_multi::handle_client_msg`) threw that text away
//! and substituted a generic `"invalid JSON command"` string. This test drives
//! the multi-client path and asserts the *detailed* text is preserved.

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
            base_dir: self._tmp.path(),
            agent: &mut self.agent,
            messages: &mut self.messages,
            conversation_snapshot: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            state_snapshot: Arc::new(tokio::sync::RwLock::new(
                self.session.state_snapshot(0, None, 0),
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
        }
    }
}

/// A malformed command line on the multi-client dispatch path must surface the
/// *detailed* parse error (the `parse error: …` serde text), matching the
/// single-client loop — not a generic `"invalid JSON command"` string (#994
/// acceptance criterion 2).
#[tokio::test]
async fn multi_client_parse_error_preserves_detailed_text() {
    let (mut fx, mut rx) = Fixture::new();
    let live = AtomicU32::new(1);

    {
        let mut ctx = fx.ctx();
        let msg = ClientMessage::Command(ClientCommand {
            line: "{not valid json".to_string(),
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

    let err = ev["error"].as_str().unwrap_or("");
    assert!(
        err.contains("parse error:"),
        "multi-client parse error must preserve the detailed serde text \
         (single-client loop emits `parse error: …`), got: {err:?}"
    );
    assert_ne!(
        err, "invalid JSON command",
        "the generic placeholder text must not be emitted (#994 criterion 2)"
    );
}

/// Golden-JSON test for criterion 3 (#994): the borrowed `MessageView`
/// serializer must produce the exact wire shape the old `json!` tree did, for
/// every role and for fully-populated tool-call fields.
#[test]
fn message_to_json_matches_golden_wire_shape_for_all_roles() {
    use crate::domain::message::ToolCall;
    use crate::interface::cli::uds_session::message_to_json;

    // A fully-populated assistant message with a tool call.
    let assistant = Message::assistant(
        "calling a tool",
        vec![ToolCall {
            id: "tc-1".to_string(),
            name: "read".to_string(),
            arguments: "{\"path\":\"x\"}".to_string(),
        }],
    );
    assert_eq!(
        message_to_json(&assistant),
        serde_json::json!({
            "role": "assistant",
            "content": "calling a tool",
            "toolCalls": [{"id": "tc-1", "name": "read", "arguments": "{\"path\":\"x\"}"}],
            "toolCallId": null,
            "toolName": null,
        })
    );

    // A tool-result message carrying toolCallId/toolName.
    let mut tool = Message::tool("tc-1", "result body");
    tool.tool_name = Some("read".to_string());
    assert_eq!(
        message_to_json(&tool),
        serde_json::json!({
            "role": "tool",
            "content": "result body",
            "toolCalls": [],
            "toolCallId": "tc-1",
            "toolName": "read",
        })
    );

    // Every role maps to its static wire name.
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

    let msgs_json: Vec<serde_json::Value> = messages.iter().map(message_to_json).collect();
    let canonical = AgentEvent::ok(
        None,
        "get_messages",
        Some(serde_json::json!({ "messages": msgs_json, "snapshot": true })),
    );
    let want: serde_json::Value =
        serde_json::from_str(&canonical.to_json_line()).expect("canonical event is JSON");

    assert_eq!(
        got, want,
        "build_get_messages_line must serialize value-identically to AgentEvent::ok"
    );
}
