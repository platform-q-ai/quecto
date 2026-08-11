//! Contract tests for the master-connection feed task seam (#1462).
//!
//! These pin the `Connection` feed-task contract the multi-session TUI
//! (epic #1467) builds on: the task owns the socket, forwards events into
//! the shared fan-in keyed `Source::Tab`, emits an explicit `Source::Closed`
//! sentinel when the stream closes, carries commands FIFO to the wire, and
//! records the ADR-0008 negotiation outcome per connection.

use super::{Connection, Source, SourcedEvent, TabId};
use crate::protocol::client::{Client, Command, Event};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

/// Bounded wait for the next fan-in item so a broken feed task fails the
/// test quickly instead of hanging it.
async fn recv_sourced(rx: &mut mpsc::Receiver<SourcedEvent>) -> Option<SourcedEvent> {
    tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .ok()
        .flatten()
}

struct Server {
    _dir: tempfile::TempDir,
    stream: tokio::net::UnixStream,
}

/// Bind a socket, connect a real framed `Client` to it, and hand back the
/// server side of the stream plus the connected client.
async fn connected_pair() -> (Client, Server) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("agent.sock");
    let listener = tokio::net::UnixListener::bind(&path).expect("bind");
    let (client, accepted) =
        tokio::join!(Client::connect(&path), async { listener.accept().await });
    let (stream, _) = accepted.expect("accept");
    (client.expect("connect"), Server { _dir: dir, stream })
}

/// #1462 scope 1+2: an event arriving on the master socket is forwarded into
/// the shared fan-in channel tagged `Source::Tab(tab)` — the event loop no
/// longer needs a dedicated `client.recv()` select arm.
#[tokio::test]
async fn connection_forwards_master_events_into_fan_in_as_tab_source() {
    let (client, mut server) = connected_pair().await;
    let (tx, mut rx) = mpsc::channel::<SourcedEvent>(16);
    let _conn = Connection::spawn(client, TabId::MASTER, tx, true);

    // Legacy NDJSON line: the client's reader sniffs framing per message.
    server
        .stream
        .write_all(b"{\"type\":\"token\",\"token\":\"fan-in-hello\"}\n")
        .await
        .expect("server write event");
    server.stream.flush().await.expect("flush");

    let item = recv_sourced(&mut rx)
        .await
        .expect("the feed task must forward the master event into the fan-in (#1462)");
    assert_eq!(
        item.0,
        Source::Tab(TabId::MASTER),
        "master events must be keyed Source::Tab(MASTER), got {:?}",
        item.0
    );
    match item.1 {
        Some(Event::Token { ref token }) => assert_eq!(
            token, "fan-in-hello",
            "the forwarded event must be the one written on the wire"
        ),
        other => panic!("expected Some(Token) payload, got {other:?}"),
    }
}

/// #1462 scope 3: stream close is an explicit `Source::Closed(tab)` sentinel
/// emitted by the feed task — not a `None`-from-recv on a dedicated arm.
#[tokio::test]
async fn connection_emits_closed_sentinel_when_stream_closes() {
    let (client, server) = connected_pair().await;
    let (tx, mut rx) = mpsc::channel::<SourcedEvent>(16);
    let _conn = Connection::spawn(client, TabId::MASTER, tx, true);

    drop(server); // Close the agent side of the socket.

    let item = recv_sourced(&mut rx)
        .await
        .expect("the feed task must emit a Closed sentinel when the stream closes (#1462)");
    assert_eq!(
        item.0,
        Source::Closed(TabId::MASTER),
        "stream close must arrive as Source::Closed(MASTER), got {:?}",
        item.0
    );
    assert!(
        item.1.is_none(),
        "the Closed sentinel carries no event payload"
    );
}

/// #1462 scope 1: commands enqueued on the connection reach the wire through
/// the feed task (the task owns the socket; callers only hold `cmd_tx`).
#[tokio::test]
async fn connection_sends_commands_through_feed_task() {
    let (client, server) = connected_pair().await;
    let (tx, _rx) = mpsc::channel::<SourcedEvent>(16);
    let conn = Connection::spawn(client, TabId::MASTER, tx, true);

    conn.try_send(&Command::GetState {
        agent_id: None,
        id: Some("seam-probe".into()),
    })
    .expect("enqueueing a command on a live connection must succeed (#1462)");

    // Framed mode writes a hello frame then the command frame; sniff the raw
    // bytes for the command payload rather than parsing framing here.
    let mut reader = BufReader::new(server.stream);
    let mut seen = String::new();
    let read = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                break;
            }
            seen.push_str(&line);
            if seen.contains("seam-probe") {
                break;
            }
        }
    })
    .await;
    assert!(
        read.is_ok() && seen.contains("seam-probe"),
        "the command must reach the wire via the feed task (#1462); saw: {seen:?}"
    );
}

/// #1462 scope 4: the ADR-0008 negotiation outcome is per-connection state —
/// a frames-speaking connection reports it.
#[tokio::test]
async fn connection_speaks_frames_true_is_per_connection_state() {
    let (client, _server) = connected_pair().await;
    let (tx, _rx) = mpsc::channel::<SourcedEvent>(16);
    let conn = Connection::spawn(client, TabId::MASTER, tx, true);
    assert!(
        conn.speaks_frames(),
        "a connection negotiated with frames must report speaks_frames() == true (#1462)"
    );
}

/// #1462 scope 4: a legacy-NDJSON connection reports NOT speaking frames —
/// the outcome is stored per connection, not a `run_tui` local.
#[tokio::test]
async fn connection_speaks_frames_false_is_per_connection_state() {
    let (client, _server) = connected_pair().await;
    let (tx, _rx) = mpsc::channel::<SourcedEvent>(16);
    let conn = Connection::spawn(client, TabId::MASTER, tx, false);
    assert!(
        !conn.speaks_frames(),
        "a legacy-NDJSON connection must report speaks_frames() == false (#1462)"
    );
}
