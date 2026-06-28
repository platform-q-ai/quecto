//! Integration tests for the connect-time snapshot push in [`spawn_accept_loop`]
//! (#874). Earlier coverage exercised the line builder
//! (`build_get_subagents_line`) and the parent reader's acceptance predicate in
//! isolation; these tests drive the REAL busy-gated accept loop over a live Unix
//! socket and assert what a freshly connected client actually receives on the
//! wire — closing the gap where the production push path
//! (`uds_multi.rs` busy branch) was only covered indirectly via the builder and
//! a hand-written BDD mock.

use super::*;
use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::infrastructure::tools::subagent_registry::{SubagentEntry, SubagentStatus};
use crate::interface::cli::protocol::SessionState;
use crate::interface::cli::uds_cancel::CancelSlot;

/// Build `AcceptLoopArgs` wired to a listener at `socket_path`, with the given
/// busy flag and subagent registry. Returns the args plus the broadcast/cmd
/// senders so the caller keeps them alive for the duration of the test.
fn make_args(
    socket_path: &std::path::Path,
    busy: bool,
    subagent_registry: Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
) -> (
    AcceptLoopArgs,
    tokio::sync::broadcast::Sender<String>,
    tokio::sync::mpsc::Sender<ClientMessage>,
    tokio::sync::mpsc::Receiver<ClientMessage>,
) {
    let listener = tokio::net::UnixListener::bind(socket_path).expect("bind test socket");
    let (broadcast_tx, _broadcast_rx) = tokio::sync::broadcast::channel::<String>(16);
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel::<ClientMessage>(16);
    let busy_flag: BusyFlag = Arc::new(std::sync::atomic::AtomicBool::new(busy));
    let args = AcceptLoopArgs {
        listener,
        broadcast_tx: broadcast_tx.clone(),
        cmd_tx: cmd_tx.clone(),
        cancel_handle: Arc::new(std::sync::Mutex::new(CancelSlot::Idle)),
        live_clients: Arc::new(AtomicU32::new(0)),
        client_tool_registry: crate::interface::cli::uds_ext_protocol::new_client_tool_registry(),
        conversation_snapshot: Arc::new(tokio::sync::RwLock::new(Vec::new())),
        state_snapshot: Arc::new(tokio::sync::RwLock::new(SessionState {
            model: "mock-model".into(),
            is_streaming: true,
            session_key: "cli:test".into(),
            message_count: 0,
            pending_message_count: 0,
            max_context_tokens: 0,
            workflow: None,
        })),
        busy: busy_flag,
        subagent_registry,
    };
    (args, broadcast_tx, cmd_tx, cmd_rx)
}

/// Read whatever bytes a freshly connected client receives within `timeout`,
/// stopping once the read either blocks past the deadline or the peer is idle.
async fn read_available(
    stream: &mut tokio::net::UnixStream,
    timeout: std::time::Duration,
) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match tokio::time::timeout(timeout, stream.read(&mut chunk)).await {
            Ok(Ok(0)) => break, // EOF
            Ok(Ok(n)) => buf.extend_from_slice(&chunk[..n]),
            Ok(Err(_)) => break, // read error
            Err(_) => break,     // no more bytes before deadline
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

/// BUSY accept loop: a newly connected client receives the connect-time
/// snapshots, INCLUDING a `get_subagents` snapshot reflecting the registry, with
/// no request sent. This is the production push at `uds_multi.rs` busy branch.
#[tokio::test]
async fn busy_accept_loop_pushes_get_subagents_snapshot() {
    let dir = tempfile::tempdir().expect("tempdir");
    let socket_path = dir.path().join("busy.sock");

    let mut entry = SubagentEntry::new("/tmp/gc.sock".into(), 4321);
    entry.status = SubagentStatus::Running;
    let registry: crate::infrastructure::tools::subagent_registry::SubagentRegistry =
        Arc::new(std::sync::Mutex::new(std::collections::HashMap::from([(
            "grandchild".to_string(),
            entry,
        )])));

    let (args, _bcast, _cmd_tx, _cmd_rx) =
        make_args(&socket_path, /* busy */ true, Some(registry));
    let handle = spawn_accept_loop(args);

    let mut client = tokio::net::UnixStream::connect(&socket_path)
        .await
        .expect("connect to busy accept loop");
    let received = read_available(&mut client, std::time::Duration::from_millis(500)).await;

    handle.abort();

    let subagents_line = received
        .lines()
        .find(|l| l.contains("\"command\":\"get_subagents\""))
        .unwrap_or_else(|| {
            panic!("busy client must receive a get_subagents snapshot; got: {received}")
        });
    let v: serde_json::Value = serde_json::from_str(subagents_line).expect("valid JSON line");
    assert_eq!(v["type"], "response");
    assert_eq!(v["command"], "get_subagents");
    assert_eq!(v["success"], true);
    assert_eq!(v["data"]["snapshot"], true);
    let agents = v["data"]["subagents"].as_array().expect("subagents array");
    assert_eq!(agents.len(), 1, "registry view served: {received}");
    assert_eq!(agents[0]["agentId"], "grandchild");

    // The state + messages snapshots are pushed too (existing #842 path).
    assert!(
        received.contains("\"command\":\"get_state\""),
        "state snapshot also pushed: {received}"
    );
    assert!(
        received.contains("\"command\":\"get_messages\""),
        "messages snapshot also pushed: {received}"
    );
}

/// IDLE accept loop: a newly connected client receives NO unsolicited bytes
/// (no get_subagents snapshot), so idle clients see no protocol change and the
/// dispatch loop answers their explicit requests in FIFO order.
#[tokio::test]
async fn idle_accept_loop_pushes_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let socket_path = dir.path().join("idle.sock");

    let registry: crate::infrastructure::tools::subagent_registry::SubagentRegistry =
        Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));

    let (args, _bcast, _cmd_tx, _cmd_rx) =
        make_args(&socket_path, /* busy */ false, Some(registry));
    let handle = spawn_accept_loop(args);

    let mut client = tokio::net::UnixStream::connect(&socket_path)
        .await
        .expect("connect to idle accept loop");
    // Send nothing; the idle loop must not write any unsolicited snapshot bytes.
    let received = read_available(&mut client, std::time::Duration::from_millis(300)).await;

    // Keep the writer half from triggering an early EOF-driven read.
    let _ = client.flush().await;
    handle.abort();

    assert!(
        received.is_empty(),
        "idle client must receive no unsolicited snapshot bytes, got: {received}"
    );
}
