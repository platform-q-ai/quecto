//! Contract for [`ReplicationSource`] (epic #2705 B11) on its board-server
//! adapter, against a scripted server on a temp socket: a batch is asked
//! of the primary's `replicate` route by revision, object and limit, and
//! read whole; a busy primary is busy, any other refusal or a malformed
//! batch refused, an absent one unreachable; the primary is named by its
//! address alone.
use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::sync::mpsc;

use quecto::application::project_board::ports::feed::{BatchLimit, OpenRefused};
use quecto::application::project_board::ports::replication::{ReplicaBatch, ReplicationSource};
use quecto::domain::project_board::value_objects::telemetry::BoardId;
use quecto::infrastructure::http::board_replica::{RemoteReplicationSource, encode_batch};
use quecto::infrastructure::http::board_service::client::BoardServiceClient;

const BOARD: &str = "0123456789abcdef";

fn batch() -> ReplicaBatch {
    ReplicaBatch {
        after: 3,
        chain_after: Some("c3".into()),
        objects: vec![],
        revisions: vec![],
        epochs: vec![],
        meta: None,
        tokens: vec![],
        tip: 3,
        more: false,
    }
}

/// A server on a temp socket answering one request with `status` and
/// `body`; the request's line is sent back.
fn scripted(
    status: &str,
    body: Vec<u8>,
) -> (
    tempfile::TempDir,
    RemoteReplicationSource,
    mpsc::Receiver<String>,
) {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("primary.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let (asked, lines) = mpsc::channel();
    let status = status.to_string();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap() == 1 {
            head.push(byte[0]);
        }
        let line = String::from_utf8_lossy(&head)
            .lines()
            .next()
            .unwrap_or_default()
            .to_string();
        let _ = asked.send(line);
        let reply = format!(
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(reply.as_bytes()).unwrap();
        stream.write_all(&body).unwrap();
    });
    let source = RemoteReplicationSource::new(
        BoardServiceClient::unix(&socket),
        BoardId::of_hex(BOARD).unwrap(),
    );
    (dir, source, lines)
}

#[test]
fn a_batch_is_asked_by_revision_object_and_limit_and_read_whole() {
    let (_dir, source, lines) = scripted("200 OK", encode_batch(&batch()).unwrap());
    let got = source.batch(3, 7, BatchLimit::of(20).unwrap());
    assert_eq!(got, Ok(batch()));
    assert_eq!(
        lines.recv().unwrap(),
        format!("GET /v1/boards/{BOARD}/replicate?after=3&objects=7&limit=20 HTTP/1.1")
    );
    assert!(
        source.primary().starts_with("unix:///"),
        "{}",
        source.primary()
    );
}

#[test]
fn refusals_are_busy_refused_or_unreachable() {
    let busy = br#"{"error":"too many","code":"busy"}"#.to_vec();
    let (_dir, source, _) = scripted("503 Service Unavailable", busy);
    assert!(matches!(
        source.batch(0, 0, BatchLimit::MOST),
        Err(OpenRefused::Busy(_))
    ));
    let gone = br#"{"error":"no such board","code":"board"}"#.to_vec();
    let (_dir, source, _) = scripted("404 Not Found", gone);
    assert!(matches!(
        source.batch(0, 0, BatchLimit::MOST),
        Err(OpenRefused::Refused(_))
    ));
    let (_dir, source, _) = scripted("200 OK", br#"{"after": 0}"#.to_vec());
    let malformed = source.batch(0, 0, BatchLimit::MOST);
    assert!(
        matches!(&malformed, Err(OpenRefused::Refused(why)) if why.contains("malformed")),
        "{malformed:?}"
    );
    let dir = tempfile::tempdir().unwrap();
    let absent = RemoteReplicationSource::new(
        BoardServiceClient::unix(&dir.path().join("none.sock")),
        BoardId::of_hex(BOARD).unwrap(),
    );
    assert!(matches!(
        absent.batch(0, 0, BatchLimit::MOST),
        Err(OpenRefused::Unreachable(_))
    ));
}
