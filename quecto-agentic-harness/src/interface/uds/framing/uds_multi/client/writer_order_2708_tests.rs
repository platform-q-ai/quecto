//! #2708 review round 3 (2): a connection's writer never puts a reply sent
//! to that client alone (a mid-turn `get_state` answer) ahead of event
//! frames broadcast before it, which the Pocket relay reads in order.
use std::sync::Arc;
use std::sync::atomic::AtomicU32;

use tokio::io::AsyncBufReadExt;

use super::super::{ClientGuard, ClientHandlerArgs, ClientMessage, handle_client};

const ITERATIONS: usize = 64;

/// Broadcast `agent_end` and `turn_outcome`, then send the client its own
/// answer, all before its writer runs; return the lines in arrival order.
async fn arrival_order() -> Vec<String> {
    let (server, client) = tokio::net::UnixStream::pair().unwrap();
    let (broadcast_tx, broadcast_rx) = tokio::sync::broadcast::channel::<String>(8);
    let (targeted_tx, targeted_rx) = tokio::sync::mpsc::channel::<String>(8);
    for frame in [r#"{"type":"agent_end"}"#, r#"{"type":"turn_outcome"}"#] {
        broadcast_tx.send(format!("{frame}\n")).unwrap();
    }
    targeted_tx
        .send("{\"type\":\"response\",\"command\":\"get_state\"}\n".to_owned())
        .await
        .unwrap();
    let (cmd_tx, _cmd_rx) = tokio::sync::mpsc::channel::<ClientMessage>(4);
    let (disconnect_tx, _disconnects) = tokio::sync::mpsc::unbounded_channel();
    let task = tokio::spawn(handle_client(ClientHandlerArgs {
        busy_state: None,
        stream: server,
        broadcast_rx,
        targeted_rx,
        cmd_tx,
        cancel_handle: Arc::new(std::sync::Mutex::new(
            crate::interface::uds::controllers::uds_cancel::CancelSlot::Idle,
        )),
        turn_control: Arc::default(),
        client_id: 7,
        client_tool_registry:
            crate::interface::uds::controllers::uds_ext_protocol::new_client_tool_registry(),
        session:
            crate::interface::uds::controllers::uds::dispatch_session_roster_tests::ephemeral_read_handles(&[]),
        subagent_registry: None,
        teardown: None,
        swarm_boards: None,
        _guard: ClientGuard {
            live_clients: Arc::new(AtomicU32::new(1)),
            disconnect_tx,
            client_id: 7,
        },
    }));
    let mut lines = tokio::io::BufReader::new(client).lines();
    let mut order = Vec::new();
    for _ in 0..3 {
        let line = tokio::time::timeout(std::time::Duration::from_secs(5), lines.next_line())
            .await
            .expect("the writer delivers")
            .unwrap()
            .expect("socket open");
        let event: serde_json::Value = serde_json::from_str(&line).unwrap();
        order.push(event["type"].as_str().unwrap().to_owned());
    }
    drop(lines);
    drop(broadcast_tx);
    task.abort();
    order
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue_2708_busy_state_a_targeted_answer_follows_frames_broadcast_before_it() {
    for iteration in 0..ITERATIONS {
        assert_eq!(
            arrival_order().await,
            ["agent_end", "turn_outcome", "response"],
            "iteration {iteration}"
        );
    }
}
