//! #2708 review rounds 3 and 4: a guarded connection's busy `get_state`
//! answer is never behind event frames broadcast before it was computed
//! (it may be ahead of frames broadcast while it was), which the Pocket
//! relay reads in order.
use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

use super::super::{ClientGuard, ClientHandlerArgs, ClientMessage, handle_client};
use crate::interface::uds::presenters::uds_busy_state::BusyState;

const ITERATIONS: usize = 64;

fn busy_source() -> BusyState {
    let session = crate::composition::bootstrap::agent_turn::build_turn_session("stub".into());
    BusyState {
        busy: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        state: Arc::new(tokio::sync::RwLock::new(
            session.state_snapshot("cli:busy", 0, None, 0, None),
        )),
        runtime_store: Default::default(),
        workflow: None,
        execution: Arc::new(std::sync::Mutex::new(Default::default())),
    }
}

struct Connection {
    _targeted: tokio::sync::mpsc::Sender<String>,
    broadcast: tokio::sync::broadcast::Sender<String>,
    client: tokio::net::UnixStream,
    task: tokio::task::JoinHandle<()>,
}

/// A guarded connection whose client already sent `get_state` and whose
/// `frames` were already broadcast, before its handler runs.
async fn connection(source: BusyState, frames: &[&str]) -> Connection {
    let (server, mut client) = tokio::net::UnixStream::pair().unwrap();
    let (broadcast, broadcast_rx) = tokio::sync::broadcast::channel::<String>(16);
    for frame in frames {
        broadcast
            .send(format!("{{\"type\":\"{frame}\"}}\n"))
            .unwrap();
    }
    client
        .write_all(b"{\"type\":\"get_state\",\"id\":\"s\"}\n")
        .await
        .unwrap();
    let (targeted, targeted_rx) = tokio::sync::mpsc::channel::<String>(8);
    let (cmd_tx, _cmd_rx) = tokio::sync::mpsc::channel::<ClientMessage>(4);
    let (disconnect_tx, _disconnects) = tokio::sync::mpsc::unbounded_channel();
    let task = tokio::spawn(handle_client(ClientHandlerArgs {
        busy_state: Some(source),
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
    Connection {
        _targeted: targeted,
        broadcast,
        client,
        task,
    }
}

/// The next `count` frames' types, in arrival order (`response` for the
/// answer, with its model appended).
async fn read(
    lines: &mut tokio::io::Lines<tokio::io::BufReader<&mut tokio::net::UnixStream>>,
    count: usize,
) -> Vec<String> {
    let mut order = Vec::new();
    for _ in 0..count {
        let line = tokio::time::timeout(Duration::from_secs(5), lines.next_line())
            .await
            .expect("the writer delivers")
            .unwrap()
            .expect("socket open");
        let event: serde_json::Value = serde_json::from_str(&line).unwrap();
        let kind = event["type"].as_str().unwrap().to_owned();
        order.push(match event["data"]["model"].as_str() {
            Some(model) => format!("{kind}:{model}"),
            None => kind,
        });
    }
    order
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue_2708_busy_state_the_answer_follows_frames_broadcast_before_it() {
    for iteration in 0..ITERATIONS {
        let mut connection = connection(busy_source(), &["agent_end", "turn_outcome"]).await;
        let mut lines = tokio::io::BufReader::new(&mut connection.client).lines();
        let order = read(&mut lines, 3).await;
        assert_eq!(
            order,
            ["agent_end", "turn_outcome", "response:stub"],
            "iteration {iteration}"
        );
        drop(lines);
        connection.task.abort();
    }
}

/// A frame broadcast while the answer is being computed may come after it,
/// and the answer reads the state as it is once computed: never behind the
/// frames sent before it, possibly ahead of that one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue_2708_busy_state_a_frame_broadcast_mid_answer_follows_it() {
    let source = busy_source();
    let snapshot = source.state.clone();
    let mut held = snapshot.write().await;
    let mut connection = connection(source, &["agent_end"]).await;
    let mut lines = tokio::io::BufReader::new(&mut connection.client).lines();
    assert_eq!(read(&mut lines, 1).await, ["agent_end"]);
    // The writer is now computing the answer, waiting on the snapshot.
    tokio::time::sleep(Duration::from_millis(100)).await;
    connection
        .broadcast
        .send("{\"type\":\"turn_outcome\"}\n".to_owned())
        .unwrap();
    held.model = "later".into();
    drop(held);
    assert_eq!(
        read(&mut lines, 2).await,
        ["response:later", "turn_outcome"]
    );
    drop(lines);
    connection.task.abort();
}
