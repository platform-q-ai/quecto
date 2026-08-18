//! Contract tests for the master-connection feed task seam (#1462).
//!
//! These pin the `Connection` feed-task contract the multi-session TUI
//! (epic #1467) builds on: the task owns the socket's event stream, forwards
//! events into the shared fan-in keyed `Source::Tab`, emits an explicit
//! `Source::Closed` sentinel when the stream closes (after every buffered
//! event), carries commands FIFO to the wire, and records the ADR-0008
//! negotiation outcome per connection — derived from the client's real
//! connect-time framing, not a caller-supplied flag.

use super::{Connection, SourcedEvent, TabId};
use crate::protocol::client::{Client, Command, Event};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
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

/// Bind a socket, connect a client to it in the given framing, and hand back
/// the server side of the stream plus the connected client.
async fn connected_pair_with(legacy: bool) -> (Client, Server) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("agent.sock");
    let listener = tokio::net::UnixListener::bind(&path).expect("bind");
    let (client, accepted) = tokio::join!(
        async {
            if legacy {
                Client::connect_legacy(&path).await
            } else {
                Client::connect(&path).await
            }
        },
        async { listener.accept().await }
    );
    let (stream, _) = accepted.expect("accept");
    (client.expect("connect"), Server { _dir: dir, stream })
}

/// Framed `Client` (protocol v2) against a raw server stream.
async fn connected_pair() -> (Client, Server) {
    connected_pair_with(false).await
}

/// A `GetState` probe command carrying a distinguishable correlation id.
fn probe(id: &str) -> Command {
    Command::GetState {
        agent_id: None,
        id: Some(id.into()),
    }
}

/// #1462 scope 1+2: an event arriving on the master socket is forwarded into
/// the shared fan-in channel tagged `Source::Tab(tab)` — the event loop no
/// longer needs a dedicated `client.recv()` select arm.
#[tokio::test]
async fn connection_forwards_master_events_into_fan_in_as_tab_source() {
    let (client, mut server) = connected_pair().await;
    let (tx, mut rx) = mpsc::channel::<SourcedEvent>(16);
    let _conn = Connection::spawn(client, TabId::MASTER, tx);

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
    match item {
        SourcedEvent::Tab(TabId::MASTER, Event::Token { ref token }) => assert_eq!(
            token, "fan-in-hello",
            "the forwarded event must be the one written on the wire"
        ),
        other => panic!("expected SourcedEvent::Tab(MASTER, Token), got {other:?}"),
    }
}

/// #1462 scope 3: stream close is an explicit `Source::Closed(tab)` sentinel
/// emitted by the feed task — not a `None`-from-recv on a dedicated arm.
#[tokio::test]
async fn connection_emits_closed_sentinel_when_stream_closes() {
    let (client, server) = connected_pair().await;
    let (tx, mut rx) = mpsc::channel::<SourcedEvent>(16);
    let _conn = Connection::spawn(client, TabId::MASTER, tx);

    drop(server); // Close the agent side of the socket.

    let item = recv_sourced(&mut rx)
        .await
        .expect("the feed task must emit a Closed sentinel when the stream closes (#1462)");
    assert!(
        matches!(item, SourcedEvent::Closed(TabId::MASTER)),
        "stream close must arrive as SourcedEvent::Closed(MASTER), got {item:?}"
    );
}

/// #1462 review (coverage): events written before the socket closes must ALL
/// arrive ahead of the `Source::Closed` sentinel — close detection must not
/// race ahead of draining buffered events, or final tokens would be lost on
/// disconnect (visible N=1 behaviour change).
#[tokio::test]
async fn events_written_before_close_arrive_before_closed_sentinel() {
    let (client, mut server) = connected_pair().await;
    let (tx, mut rx) = mpsc::channel::<SourcedEvent>(16);
    let _conn = Connection::spawn(client, TabId::MASTER, tx);

    server
        .stream
        .write_all(b"{\"type\":\"token\",\"token\":\"final-token\"}\n")
        .await
        .expect("server write event");
    server.stream.flush().await.expect("flush");
    drop(server); // Immediately close with the event still in flight.

    let first = recv_sourced(&mut rx)
        .await
        .expect("the in-flight event must still be delivered (#1462)");
    match first {
        SourcedEvent::Tab(TabId::MASTER, Event::Token { ref token }) => {
            assert_eq!(token, "final-token")
        }
        other => panic!("expected the buffered Token before the sentinel, got {other:?}"),
    }
    let second = recv_sourced(&mut rx)
        .await
        .expect("the Closed sentinel must follow the buffered event (#1462)");
    assert!(
        matches!(second, SourcedEvent::Closed(TabId::MASTER)),
        "the Closed sentinel must follow the buffered event, got {second:?}"
    );
}

/// #1462 scope 1: commands enqueued on the connection reach the wire through
/// the ordered writer task in enqueue (FIFO) order — callers hold only the
/// connection handle; the socket lives behind the connection's tasks.
#[tokio::test]
async fn connection_sends_commands_through_feed_task_in_fifo_order() {
    let (client, server) = connected_pair().await;
    let (tx, _rx) = mpsc::channel::<SourcedEvent>(16);
    let conn = Connection::spawn(client, TabId::MASTER, tx);

    for id in ["seam-1", "seam-2", "seam-3"] {
        conn.try_send(&probe(id))
            .expect("enqueueing a command on a live connection must succeed (#1462)");
    }

    // Framed mode writes a hello frame then the command frames (no
    // newlines); sniff the raw bytes for the command payloads rather than
    // parsing framing here.
    let mut stream = server.stream;
    let mut seen = String::new();
    let read = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        let mut buf = [0u8; 4096];
        loop {
            let n = stream.read(&mut buf).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            seen.push_str(&String::from_utf8_lossy(&buf[..n]));
            if seen.contains("seam-3") {
                break;
            }
        }
    })
    .await;
    assert!(
        read.is_ok(),
        "all three commands must reach the wire via the feed task (#1462); saw: {seen:?}"
    );
    let positions: Vec<usize> = ["seam-1", "seam-2", "seam-3"]
        .iter()
        .map(|id| {
            seen.find(id)
                .unwrap_or_else(|| panic!("{id} must reach the wire; saw: {seen:?}"))
        })
        .collect();
    assert!(
        positions[0] < positions[1] && positions[1] < positions[2],
        "commands must appear on the wire in enqueue (FIFO) order (#1462); saw: {seen:?}"
    );
}

/// #1462 scope 4: the ADR-0008 outcome is per-connection state derived from
/// the client's REAL connect-time framing (not a caller flag), and matches
/// the observable wire format: a framed connection announces itself with the
/// length-prefixed empty hello frame.
#[tokio::test]
async fn frames_connection_reports_negotiation_and_writes_framed_hello() {
    let (client, mut server) = connected_pair().await;
    let (tx, _rx) = mpsc::channel::<SourcedEvent>(16);
    let conn = Connection::spawn(client, TabId::MASTER, tx);
    assert!(
        conn.speaks_frames(),
        "a connection built from a framed client must report speaks_frames() (#1462)"
    );

    // Observable wire behaviour: framed mode's first bytes are the empty
    // hello frame's zero length prefix — not printable NDJSON.
    let mut prefix = [0u8; 4];
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        server.stream.read_exact(&mut prefix),
    )
    .await
    .expect("hello must arrive")
    .expect("read hello prefix");
    assert_eq!(
        prefix,
        [0, 0, 0, 0],
        "a frames-speaking connection must open with the empty hello frame (ADR-0008)"
    );
}

/// #1462 scope 4: a legacy-NDJSON connection reports NOT speaking frames and
/// writes newline-delimited JSON commands — the outcome is stored per
/// connection, derived from the real `connect_legacy` negotiation.
#[tokio::test]
async fn legacy_connection_reports_negotiation_and_writes_ndjson() {
    let (client, server) = connected_pair_with(true).await;
    let (tx, _rx) = mpsc::channel::<SourcedEvent>(16);
    let conn = Connection::spawn(client, TabId::MASTER, tx);
    assert!(
        !conn.speaks_frames(),
        "a connection built from a legacy client must report !speaks_frames() (#1462)"
    );

    conn.try_send(&probe("legacy-probe"))
        .expect("enqueue on a live legacy connection");
    let mut reader = BufReader::new(server.stream);
    let mut line = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        reader.read_line(&mut line),
    )
    .await
    .expect("legacy command must arrive")
    .expect("read legacy line");
    assert!(
        line.ends_with('\n') && serde_json::from_str::<serde_json::Value>(&line).is_ok(),
        "legacy mode must write one newline-terminated JSON command, got: {line:?}"
    );
    assert!(
        line.contains("legacy-probe"),
        "the NDJSON line must carry the enqueued command, got: {line:?}"
    );
}

/// #1465 F1: dropping a Connection must abort its feed task so a recycled
/// TabId cannot receive a late Closed sentinel from the previous occupant.
#[tokio::test]
async fn connection_drop_aborts_feed_task() {
    let (client, server) = connected_pair().await;
    let (tx, mut rx) = mpsc::channel::<SourcedEvent>(16);
    let conn = Connection::spawn(client, TabId::MASTER, tx);
    drop(conn);
    // Drop the server side so a non-aborted feed would eventually emit Closed.
    drop(server);
    let closed = tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await;
    // Aborted feed should not deliver Closed (or anything). Either timeout or
    // channel closed without a Closed sentinel is acceptable; a Closed event is not.
    if let Ok(Some(item)) = closed {
        assert!(
            !matches!(item, SourcedEvent::Closed(_)),
            "F1: drop must abort feed before Closed can poison the fan-in"
        );
    }
}
