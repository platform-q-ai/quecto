//! #843: an agent-targeted `get_messages` with NO `count` must forward to the
//! named child and return ITS full history — it must never fall through to the
//! local fast path, which ignores `agent_id` and would silently answer from the
//! connected/parent agent's own conversation.
//!
//! Self-contained (own minimal `DispatchCtx`) so it stays independent of the
//! larger `cov_tests` fixture and keeps each file within the size budget.
use super::{
    ForwardGetMessage, dispatch_command, forward_subagent_get_message,
    forward_subagent_get_messages,
};
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::domain::message::Message;
use crate::infrastructure::persistence::session_store::FileSessionStore;
use crate::infrastructure::tools::subagent_registry::{
    SubagentEntry, SubagentRegistry, new_registry,
};
use crate::interface::cli::protocol::AgentCommand;
use crate::interface::cli::uds::DispatchCtx;
use crate::interface::cli::uds_cancel::CancelSlot;
use crate::interface::cli::uds_ext_protocol::new_client_tool_registry;
use crate::interface::cli::uds_session::AgentSession;

pub(super) struct Fx {
    agent: AgentLoopImpl,
    pub(super) messages: Vec<Message>,
    session: AgentSession,
    session_key: String,
    store: FileSessionStore,
    _tmp: tempfile::TempDir,
    writer: tokio::io::Sink,
}

impl Fx {
    pub(super) fn new() -> Self {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = FileSessionStore::new(tmp.path());
        Self {
            agent: AgentLoopImpl::new(AgentLoopConfig {
                provider: crate::interface::test_support::make_stub_provider(),
                tool_registry: Box::new(
                    crate::infrastructure::tools::registry::ToolRegistryImpl::new(),
                ),
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
                audit_log: None,
                pin_recent_turns: 2,
                context_collapse_after_messages: u32::MAX,
                model_context_window: None,
            }),
            messages: Vec::new(),
            session: AgentSession::new("stub".into(), "cli:test".into()),
            session_key: "cli:test".into(),
            store,
            _tmp: tmp,
            writer: tokio::io::sink(),
        }
    }

    pub(super) fn ctx(&mut self) -> DispatchCtx<'_> {
        let initial_stats = crate::interface::cli::uds_session::compute_session_stats(
            &self.session_key,
            &self.messages,
        );
        DispatchCtx {
            execution_state: std::sync::Arc::new(std::sync::Mutex::new(Default::default())),
            wire_mode: crate::interface::cli::uds_wire::ConnectionWireMode::legacy(),
            base_dir: self._tmp.path(),
            agent: &mut self.agent,
            messages: &mut self.messages,
            conversation_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(
                crate::interface::cli::uds_snapshots::ConversationSnapshotData::default(),
            )),
            state_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(
                self.session.state_snapshot(0, None, 0, None),
            )),
            session_stats_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(initial_stats)),
            tool_catalogue_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
            busy: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            session: &mut self.session,
            stdout: Some(&mut self.writer),
            session_key: &mut self.session_key,
            session_store: &self.store,
            ephemeral: false,
            system_prompt: "",
            cancel_handle: std::sync::Arc::new(std::sync::Mutex::new(CancelSlot::Idle)),
            turn_control: std::sync::Arc::default(),
            broadcast_tx: None,
            _ext_registry: None,
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

/// A bare-bones child UDS server: records the first command line received and
/// replies with an id-correlated `get_messages` response carrying `marker`.
pub(super) async fn spawn_recording_child(
    marker: &'static str,
) -> (
    std::path::PathBuf,
    std::sync::Arc<tokio::sync::Mutex<String>>,
    tempfile::TempDir,
    tokio::task::JoinHandle<()>,
) {
    use tokio::io::{AsyncWriteExt, BufReader};
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("child.sock");
    let listener = tokio::net::UnixListener::bind(&sock).unwrap();
    let received = std::sync::Arc::new(tokio::sync::Mutex::new(String::new()));
    let recv2 = received.clone();
    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut write_half) = tokio::io::split(stream);
        let mut reader = BufReader::new(read_half);
        let line = crate::infrastructure::test_support::read_framed_command_async(&mut reader)
            .await
            .unwrap();
        *recv2.lock().await = line.clone();
        let req: serde_json::Value = serde_json::from_str(&line).unwrap();
        let id = req.get("id").and_then(|v| v.as_str()).unwrap();
        let reply = format!(
            "{{\"type\":\"response\",\"id\":\"{id}\",\"command\":\"get_messages\",\"success\":true,\"data\":{{\"messages\":[{{\"content\":\"{marker}\"}}]}}}}\n"
        );
        write_half.write_all(reply.as_bytes()).await.unwrap();
        write_half.flush().await.unwrap();
    });
    (sock, received, dir, handle)
}

pub(super) fn register_child(registry: &SubagentRegistry, id: &str, sock: std::path::PathBuf) {
    registry
        .lock()
        .unwrap()
        .insert(id.to_string(), SubagentEntry::new(sock, 0));
}

pub(super) async fn spawn_recording_get_message_child(
    marker: &'static str,
) -> (
    std::path::PathBuf,
    std::sync::Arc<tokio::sync::Mutex<String>>,
    tempfile::TempDir,
    tokio::task::JoinHandle<()>,
) {
    use tokio::io::{AsyncWriteExt, BufReader};
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("child-get-message-dispatch.sock");
    let listener = tokio::net::UnixListener::bind(&sock).unwrap();
    let received = std::sync::Arc::new(tokio::sync::Mutex::new(String::new()));
    let recv2 = received.clone();
    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut write_half) = tokio::io::split(stream);
        let mut reader = BufReader::new(read_half);
        let line = crate::infrastructure::test_support::read_framed_command_async(&mut reader)
            .await
            .unwrap();
        *recv2.lock().await = line.clone();
        let req: serde_json::Value = serde_json::from_str(&line).unwrap();
        let id = req.get("id").and_then(|v| v.as_str()).unwrap();
        let reply = format!(
            "{{\"type\":\"response\",\"id\":\"{id}\",\"command\":\"get_message\",\"success\":true,\"data\":{{\"id\":\"child-message\",\"content\":\"{marker}\"}}}}\n"
        );
        write_half.write_all(reply.as_bytes()).await.unwrap();
        write_half.flush().await.unwrap();
    });
    (sock, received, dir, handle)
}

pub(super) async fn spawn_recording_sync_child(
    marker: &'static str,
) -> (
    std::path::PathBuf,
    std::sync::Arc<tokio::sync::Mutex<String>>,
    tempfile::TempDir,
    tokio::task::JoinHandle<()>,
) {
    use tokio::io::{AsyncWriteExt, BufReader};
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("child-sync.sock");
    let listener = tokio::net::UnixListener::bind(&sock).unwrap();
    let received = std::sync::Arc::new(tokio::sync::Mutex::new(String::new()));
    let recv2 = received.clone();
    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut write_half) = tokio::io::split(stream);
        let mut reader = BufReader::new(read_half);
        let line = crate::infrastructure::test_support::read_framed_command_async(&mut reader)
            .await
            .unwrap();
        *recv2.lock().await = line.clone();
        let req: serde_json::Value = serde_json::from_str(&line).unwrap();
        let id = req.get("id").and_then(|v| v.as_str()).unwrap();
        let reply = format!(
            "{{\"type\":\"response\",\"id\":\"{id}\",\"command\":\"sync\",\"success\":true,\"data\":{{\"epoch\":4,\"rev\":3,\"changes\":[{{\"id\":\"{marker}\"}}]}}}}\n"
        );
        write_half.write_all(reply.as_bytes()).await.unwrap();
        write_half.flush().await.unwrap();
    });
    (sock, received, dir, handle)
}

pub(super) async fn spawn_recording_replying_child(
    reply: &'static str,
) -> (
    std::path::PathBuf,
    std::sync::Arc<tokio::sync::Mutex<String>>,
    tempfile::TempDir,
    tokio::task::JoinHandle<()>,
) {
    use tokio::io::{AsyncWriteExt, BufReader};
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("child-recording-reply.sock");
    let listener = tokio::net::UnixListener::bind(&sock).unwrap();
    let received = std::sync::Arc::new(tokio::sync::Mutex::new(String::new()));
    let recv2 = received.clone();
    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut write_half) = tokio::io::split(stream);
        let mut reader = BufReader::new(read_half);
        let command = crate::infrastructure::test_support::read_framed_command_async(&mut reader)
            .await
            .unwrap();
        *recv2.lock().await = command.clone();
        let request: serde_json::Value = serde_json::from_str(&command).unwrap();
        let request_id = request.get("id").and_then(|value| value.as_str()).unwrap();
        let reply = reply.replace("__ID__", request_id);
        write_half.write_all(reply.as_bytes()).await.unwrap();
        write_half.flush().await.unwrap();
    });
    (sock, received, dir, handle)
}

pub(super) async fn spawn_replying_child(
    reply: &'static str,
) -> (
    std::path::PathBuf,
    tempfile::TempDir,
    tokio::task::JoinHandle<()>,
) {
    use tokio::io::{AsyncWriteExt, BufReader};
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("child.sock");
    let listener = tokio::net::UnixListener::bind(&sock).unwrap();
    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut write_half) = tokio::io::split(stream);
        let mut reader = BufReader::new(read_half);
        let command = crate::infrastructure::test_support::read_framed_command_async(&mut reader)
            .await
            .unwrap();
        let request: serde_json::Value = serde_json::from_str(&command).unwrap();
        let request_id = request.get("id").and_then(|value| value.as_str()).unwrap();
        let reply = reply.replace("__ID__", request_id);
        write_half.write_all(reply.as_bytes()).await.unwrap();
        write_half.flush().await.unwrap();
    });
    (sock, dir, handle)
}

#[tokio::test]
async fn dispatch_uncounted_agent_targeted_get_messages_forwards_to_child() {
    let (sock, received, _dir, handle) = spawn_recording_child("CHILD_HISTORY").await;
    let registry = new_registry();
    register_child(&registry, "worker", sock);

    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(8);
    let mut fx = Fx::new();
    // Parent has its OWN distinct history that must NOT leak into the reply.
    fx.messages.push(Message::user("PARENT_ONLY"));
    {
        let mut ctx = fx.ctx();
        ctx.subagent_registry = Some(registry);
        ctx.broadcast_tx = Some(tx);
        let cmd = AgentCommand::GetMessages {
            id: Some("q1".into()),
            count: None,
            before: None,
            agent_id: Some("worker".into()),
        };
        assert!(!dispatch_command(cmd, &mut ctx).await);
    }
    handle.await.unwrap();

    let emitted = rx.try_recv().expect("a response event should be emitted");
    assert!(
        emitted.contains("CHILD_HISTORY"),
        "must return the child's history, got: {emitted}"
    );
    assert!(
        !emitted.contains("PARENT_ONLY"),
        "must NOT leak the parent's conversation, got: {emitted}"
    );
    // The forwarded command requests FULL history (no `count`).
    let fwd = received.lock().await.clone();
    let fwd_json: serde_json::Value = serde_json::from_str(&fwd).unwrap();
    assert_eq!(fwd_json["type"], "get_messages");
    assert!(
        fwd_json.get("count").is_none(),
        "uncounted request must forward without a count, got: {fwd}"
    );
}

#[tokio::test]
async fn dispatch_agent_targeted_get_messages_forwards_before_cursor_to_child() {
    let (sock, received, _dir, handle) = spawn_recording_child("OLDER_CHILD_HISTORY").await;
    let registry = new_registry();
    register_child(&registry, "worker", sock);

    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(8);
    let mut fx = Fx::new();
    fx.messages.push(Message::user("PARENT_ONLY"));
    {
        let mut ctx = fx.ctx();
        ctx.subagent_registry = Some(registry);
        ctx.broadcast_tx = Some(tx);
        let cmd = AgentCommand::GetMessages {
            id: Some("q-before".into()),
            count: None,
            before: Some("child-cursor".into()),
            agent_id: Some("worker".into()),
        };
        assert!(!dispatch_command(cmd, &mut ctx).await);
    }
    handle.await.unwrap();

    let emitted = rx.try_recv().expect("a response event should be emitted");
    assert!(emitted.contains("OLDER_CHILD_HISTORY"), "got: {emitted}");
    assert!(!emitted.contains("PARENT_ONLY"), "got: {emitted}");
    let fwd = received.lock().await.clone();
    let fwd_json: serde_json::Value = serde_json::from_str(&fwd).unwrap();
    assert_eq!(fwd_json["type"], "get_messages");
    assert_eq!(
        fwd_json["before"], "child-cursor",
        "targeted older-page request must preserve the child cursor, got: {fwd}"
    );
}

#[tokio::test]
async fn dispatch_counted_agent_targeted_get_messages_forwards_count_to_child() {
    // Regression twin of the uncounted case: a `count: Some(n)` agent-targeted
    // request must forward `{"type":"get_messages","count":n}` to the child and
    // return the child's history — guarding the `Some(count)` serialization arm.
    let (sock, received, _dir, handle) = spawn_recording_child("CHILD_HISTORY").await;
    let registry = new_registry();
    register_child(&registry, "worker", sock);

    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(8);
    let mut fx = Fx::new();
    fx.messages.push(Message::user("PARENT_ONLY"));
    {
        let mut ctx = fx.ctx();
        ctx.subagent_registry = Some(registry);
        ctx.broadcast_tx = Some(tx);
        let cmd = AgentCommand::GetMessages {
            id: Some("q1".into()),
            count: Some(7),
            before: None,
            agent_id: Some("worker".into()),
        };
        assert!(!dispatch_command(cmd, &mut ctx).await);
    }
    handle.await.unwrap();

    let emitted = rx.try_recv().expect("a response event should be emitted");
    assert!(
        emitted.contains("CHILD_HISTORY"),
        "must return the child's history, got: {emitted}"
    );
    assert!(
        !emitted.contains("PARENT_ONLY"),
        "must NOT leak the parent's conversation, got: {emitted}"
    );
    let fwd = received.lock().await.clone();
    let fwd_json: serde_json::Value = serde_json::from_str(&fwd).unwrap();
    assert_eq!(fwd_json["type"], "get_messages");
    assert_eq!(
        fwd_json["count"], 7,
        "counted request must forward its count to the child, got: {fwd}"
    );
}

#[tokio::test]
async fn forward_get_messages_propagates_child_failure() {
    let (sock, _dir, handle) = spawn_replying_child(
        "{\"type\":\"response\",\"id\":\"__ID__\",\"command\":\"get_messages\",\"success\":false,\"error\":\"history cursor not found: stale\"}\n",
    )
    .await;
    let registry = new_registry();
    register_child(&registry, "worker", sock);
    let mut fx = Fx::new();
    let mut ctx = fx.ctx();
    ctx.subagent_registry = Some(registry);

    let event = forward_subagent_get_messages(
        &ctx,
        Some(crate::domain::ids::CommandId::from("page-2")),
        "get_messages",
        crate::domain::ids::AgentId::from("worker"),
        None,
        Some(crate::domain::ids::MessageId::from("stale")),
    )
    .await;
    handle.await.unwrap();
    let json = serde_json::to_value(event).unwrap();

    assert_eq!(json["success"], false);
    assert_eq!(json["error"], "history cursor not found: stale");
}

#[tokio::test]
async fn forward_get_messages_rejects_malformed_child_response() {
    let (sock, _dir, handle) = spawn_replying_child("not-json\n").await;
    let registry = new_registry();
    register_child(&registry, "worker", sock);
    let mut fx = Fx::new();
    let mut ctx = fx.ctx();
    ctx.subagent_registry = Some(registry);

    let event = forward_subagent_get_messages(
        &ctx,
        Some(crate::domain::ids::CommandId::from("page-2")),
        "get_messages",
        crate::domain::ids::AgentId::from("worker"),
        None,
        None,
    )
    .await;
    handle.await.unwrap();
    let json = serde_json::to_value(event).unwrap();

    assert_eq!(json["success"], false);
    assert!(
        json["error"]
            .as_str()
            .is_some_and(|error| !error.is_empty()),
        "malformed child JSON must be surfaced as an error: {json}"
    );
}

#[tokio::test]
async fn forward_uncounted_no_registry_is_error_event() {
    let mut fx = Fx::new();
    let ctx = fx.ctx(); // subagent_registry: None
    let ev = forward_subagent_get_messages(
        &ctx,
        Some(crate::domain::ids::CommandId::from("id1")),
        "get_messages",
        crate::domain::ids::AgentId::from("worker"),
        None,
        None,
    )
    .await;
    let json = serde_json::to_value(&ev).unwrap();
    assert!(
        json.get("error").is_some(),
        "missing registry must surface an error: {json}"
    );
}

#[tokio::test]
async fn forward_tail_unknown_agent_is_error_event() {
    let mut fx = Fx::new();
    let mut ctx = fx.ctx();
    ctx.subagent_registry = Some(new_registry());
    let ev = forward_subagent_get_messages(
        &ctx,
        Some(crate::domain::ids::CommandId::from("id1")),
        "get_messages_tail",
        crate::domain::ids::AgentId::from("ghost"),
        Some(3),
        None,
    )
    .await;
    let json = serde_json::to_value(&ev).unwrap();
    let err = json.get("error").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        err.contains("not found"),
        "unknown agent must report not-found: {json}"
    );
}

// #1060: the singular `forward_subagent_get_message` (child ref lookup) must
// share the plural's error semantics.
#[tokio::test]
async fn forward_get_message_preserves_id_and_range_on_child_wire() {
    use tokio::io::{AsyncWriteExt, BufReader};
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("child-get-message.sock");
    let listener = tokio::net::UnixListener::bind(&sock).unwrap();
    let received = std::sync::Arc::new(tokio::sync::Mutex::new(String::new()));
    let recorded = received.clone();
    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut write_half) = tokio::io::split(stream);
        let mut reader = BufReader::new(read_half);
        let line = crate::infrastructure::test_support::read_framed_command_async(&mut reader)
            .await
            .unwrap();
        *recorded.lock().await = line.clone();
        let request: serde_json::Value = serde_json::from_str(&line).unwrap();
        let child_id = request["id"].as_str().unwrap();
        let reply = format!(
            "{{\"type\":\"response\",\"id\":\"{child_id}\",\"command\":\"get_message\",\"success\":true,\"data\":{{\"id\":\"m1\",\"content\":\"page\",\"offset\":4096,\"nextOffset\":4100,\"contentLength\":4100,\"hasMoreContent\":false}}}}\n"
        );
        write_half.write_all(reply.as_bytes()).await.unwrap();
    });
    let registry = new_registry();
    register_child(&registry, "worker", sock);
    let mut fx = Fx::new();
    let mut ctx = fx.ctx();
    ctx.subagent_registry = Some(registry);

    let event = forward_subagent_get_message(
        &ctx,
        Some("parent-page"),
        "get_message",
        ForwardGetMessage {
            agent_id: crate::domain::ids::AgentId::from("worker"),
            message_id: crate::domain::ids::MessageId::from("m1"),
            tool_call_id: Some(crate::domain::ids::ToolCallId::from("call-large")),
            offset: Some(4096),
            limit: Some(8192),
        },
    )
    .await;
    handle.await.unwrap();

    let command: serde_json::Value = serde_json::from_str(&received.lock().await).unwrap();
    assert!(
        command["id"].as_str().is_some_and(|id| !id.is_empty()),
        "forwarding transport must stamp a child correlation id"
    );
    assert_eq!(command["messageId"], "m1");
    assert_eq!(command["toolCallId"], "call-large");
    assert_eq!(command["offset"], 4096);
    assert_eq!(command["limit"], 8192);
    let response = serde_json::to_value(event).unwrap();
    assert_eq!(response["success"], true);
    assert_eq!(response["id"], "parent-page");
}

#[tokio::test]
async fn forward_get_message_no_registry_is_error_event() {
    let mut fx = Fx::new();
    let ctx = fx.ctx(); // subagent_registry: None
    let ev = forward_subagent_get_message(
        &ctx,
        Some("id1"),
        "get_message",
        ForwardGetMessage {
            agent_id: crate::domain::ids::AgentId::from("worker"),
            message_id: crate::domain::ids::MessageId::from("m1"),
            tool_call_id: None,
            offset: None,
            limit: None,
        },
    )
    .await;
    let json = serde_json::to_value(&ev).unwrap();
    assert!(
        json.get("error").is_some(),
        "missing registry must surface an error: {json}"
    );
}

#[tokio::test]
async fn forward_get_message_unknown_agent_is_error_event() {
    let mut fx = Fx::new();
    let mut ctx = fx.ctx();
    ctx.subagent_registry = Some(new_registry());
    let ev = forward_subagent_get_message(
        &ctx,
        Some("id1"),
        "get_message",
        ForwardGetMessage {
            agent_id: crate::domain::ids::AgentId::from("ghost"),
            message_id: crate::domain::ids::MessageId::from("m1"),
            tool_call_id: None,
            offset: None,
            limit: None,
        },
    )
    .await;
    let json = serde_json::to_value(&ev).unwrap();
    let err = json.get("error").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        err.contains("not found"),
        "unknown agent must report not-found: {json}"
    );
}
