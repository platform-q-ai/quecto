//! Contract for [`BoardWatch`] (epic #2705 B10), on the remote watch over
//! canned event streams: it hands on each event, opens the stream again
//! from the last id when it ends, so nothing repeats; a refusal is said.
use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::sync::mpsc;
use std::time::Duration;

use quecto::application::project_board::ports::feed::{
    BoardRevision, BoardWatch, FeedEvent, WatchFlow,
};
use quecto::domain::project_board::value_objects::telemetry::BoardId;
use quecto::infrastructure::http::board_service::client::BoardServiceClient;
use quecto::infrastructure::http::board_watch::RemoteBoardWatch;

const BOARD: &str = "0123456789abcdef";

fn sse(body: &str) -> Vec<u8> {
    let mut reply = b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\
                      transfer-encoding: chunked\r\nconnection: close\r\n\r\n"
        .to_vec();
    reply.extend(format!("{:x}\r\n{body}\r\n0\r\n\r\n", body.len()).as_bytes());
    reply
}

fn revision(id: u64) -> String {
    format!(
        "id: {id}\nevent: revision\ndata: {{\"kind\":\"revision\",\"epoch\":1,\"revision\":{id}}}\n\n"
    )
}

/// Serves `replies` one connection each on a temp socket; sends each head
/// the watch sent.
fn canned(replies: Vec<Vec<u8>>) -> (tempfile::TempDir, RemoteBoardWatch, mpsc::Receiver<String>) {
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
    let watch = RemoteBoardWatch::timed(
        BoardServiceClient::unix(&socket),
        BoardId::of_hex(BOARD).expect("a board id"),
        Duration::from_secs(5),
        Duration::from_millis(10),
    );
    (dir, watch, heard)
}

#[test]
fn an_ended_stream_is_opened_again_from_its_last_id_and_nothing_repeats() {
    let first = format!(
        "retry: 1000\n\nid: 0\nevent: epoch\ndata: {{\"kind\":\"epoch\",\"epoch\":1}}\n\n{}{}",
        revision(1),
        revision(2)
    );
    let (_dir, watch, heads) = canned(vec![sse(&first), sse(&revision(3))]);
    let mut told = Vec::new();
    let watched = watch.watch(Some(0), &mut |event| {
        told.push(event);
        match event {
            FeedEvent::Revision(BoardRevision { revision: 3, .. }) => WatchFlow::Stop,
            _ => WatchFlow::Continue,
        }
    });
    assert_eq!(watched, Ok(()));
    let revisions: Vec<u64> = told
        .iter()
        .filter_map(|event| match event {
            FeedEvent::Revision(revision) => Some(revision.revision),
            _ => None,
        })
        .collect();
    assert_eq!(revisions, vec![1, 2, 3]);
    assert_eq!(told[0], FeedEvent::Epoch { epoch: 1 });
    let first_head = heads.recv_timeout(Duration::from_secs(5)).expect("a head");
    assert!(first_head.starts_with(&format!("GET /v1/boards/{BOARD}/events HTTP/1.1\r\n")));
    assert!(first_head.contains("last-event-id: 0\r\n"), "{first_head}");
    let second_head = heads.recv_timeout(Duration::from_secs(5)).expect("a head");
    assert!(
        second_head.contains("last-event-id: 2\r\n"),
        "{second_head}"
    );
}

#[test]
fn a_refusal_ends_the_watch_saying_why() {
    let body = r#"{"error":"unauthorized","code":"token"}"#;
    let reply = format!(
        "HTTP/1.1 401 Unauthorized\r\ncontent-type: application/json\r\n\
         content-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let (_dir, watch, _) = canned(vec![reply.into_bytes()]);
    let watched = watch.watch(None, &mut |_| WatchFlow::Continue);
    let why = watched.expect_err("a refusal ends it");
    assert!(why.contains("401 token"), "{why}");
}
