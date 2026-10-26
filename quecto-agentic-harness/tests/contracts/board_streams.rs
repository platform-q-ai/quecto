//! Contract for [`BoardStreams`] (epic #2705 B10), on the remote feed over
//! canned replies: a stream opens from its cursor; only a 503 `busy` is
//! busy, any other refusal is a refusal; no server is unreachable.
use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::sync::mpsc;
use std::time::Duration;

use quecto::application::project_board::ports::feed::{
    BoardRevision, BoardStream, BoardStreams, FeedEvent, OpenRefused, StreamRead,
};
use quecto::domain::project_board::value_objects::telemetry::BoardId;
use quecto::infrastructure::http::board_service::client::BoardServiceClient;
use quecto::infrastructure::http::board_watch::RemoteBoardStreams;

pub(super) const BOARD: &str = "0123456789abcdef";

/// A chunked event-stream reply of `body`; `last` ends it with its last
/// chunk, else the connection just closes at a chunk boundary.
pub(super) fn sse(body: &str, last: bool) -> Vec<u8> {
    let mut reply = b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\
                      transfer-encoding: chunked\r\nconnection: close\r\n\r\n"
        .to_vec();
    if !body.is_empty() {
        reply.extend(format!("{:x}\r\n{body}\r\n", body.len()).as_bytes());
    }
    if last {
        reply.extend(b"0\r\n\r\n");
    }
    reply
}

/// A JSON refusal of `status` and `code`.
pub(super) fn refusal(status: u16, code: &str) -> Vec<u8> {
    let body = format!(r#"{{"error":"why it was refused","code":"{code}"}}"#);
    format!(
        "HTTP/1.1 {status} Refused\r\ncontent-type: application/json\r\n\
         content-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

pub(super) fn revision(id: u64) -> String {
    format!(
        "id: {id}\nevent: revision\ndata: {{\"kind\":\"revision\",\"epoch\":1,\"revision\":{id}}}\n\n"
    )
}

/// Serves `replies` one connection each on a temp socket; sends each head
/// asked.
pub(super) fn canned(
    replies: Vec<Vec<u8>>,
) -> (
    tempfile::TempDir,
    RemoteBoardStreams,
    mpsc::Receiver<String>,
) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let socket = dir.path().join("board.sock");
    let listener = UnixListener::bind(&socket).expect("a socket");
    let (heads, heard) = mpsc::channel();
    std::thread::spawn(move || {
        for reply in replies {
            let (mut stream, _) = listener.accept().expect("a connection");
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
                head.push(byte[0]);
            }
            let _ = heads.send(String::from_utf8_lossy(&head).into_owned());
            let _ = stream.write_all(&reply);
        }
    });
    let streams = RemoteBoardStreams::idling(
        BoardServiceClient::unix(&socket),
        BoardId::of_hex(BOARD).expect("a board id"),
        Duration::from_secs(5),
    );
    (dir, streams, heard)
}

/// Every read of `stream` to its end (or a malformed read).
pub(super) fn reads(stream: &mut Box<dyn BoardStream>) -> Vec<StreamRead> {
    let mut all = Vec::new();
    loop {
        let read = stream.read();
        let last = !matches!(read, StreamRead::Event { .. });
        all.push(read);
        if last {
            return all;
        }
    }
}

fn opened(stream: Result<Box<dyn BoardStream>, OpenRefused>) -> Box<dyn BoardStream> {
    stream.unwrap_or_else(|refused| panic!("it opens: {refused:?}"))
}

#[test]
fn a_stream_opens_from_its_cursor() {
    let (_dir, streams, heads) = canned(vec![sse(&revision(3), true)]);
    let mut stream = opened(streams.open(Some(2)));
    let head = heads.recv_timeout(Duration::from_secs(5)).expect("a head");
    assert!(head.starts_with(&format!("GET /v1/boards/{BOARD}/events HTTP/1.1\r\n")));
    assert!(head.contains("last-event-id: 2\r\n"), "{head}");
    let event = FeedEvent::Revision(BoardRevision {
        epoch: 1,
        revision: 3,
    });
    assert_eq!(
        reads(&mut stream),
        vec![StreamRead::Event { id: 3, event }, StreamRead::Ended]
    );
}

/// Review (#2757 round 3, M1): only a 503 whose code is `busy` is busy. A
/// 503 `unavailable` (a store or token store down), `clock_unsynced`, or
/// any other refusal is a refusal, which ends the watch at once.
#[test]
fn only_a_503_busy_is_busy() {
    let (_dir, streams, _) = canned(vec![
        refusal(503, "busy"),
        refusal(503, "unavailable"),
        refusal(503, "clock_unsynced"),
        refusal(401, "token"),
    ]);
    let busy = streams.open(None).err().expect("refused");
    assert!(
        matches!(&busy, OpenRefused::Busy(why) if why.contains("503 busy")),
        "{busy:?}"
    );
    for code in ["503 unavailable", "503 clock_unsynced", "401 token"] {
        let refused = streams.open(None).err().expect("refused");
        let said = matches!(&refused, OpenRefused::Refused(why) if why.contains(code));
        assert!(said, "{code}: {refused:?}");
    }
}

#[test]
fn no_server_is_unreachable() {
    let (dir, streams, _) = canned(vec![]);
    std::fs::remove_file(dir.path().join("board.sock")).expect("the socket goes");
    assert!(matches!(
        streams.open(None),
        Err(OpenRefused::Unreachable(_))
    ));
}
