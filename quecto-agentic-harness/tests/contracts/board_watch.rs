//! Contract for [`BoardWatch`] (epic #2705 B10), on the remote watch over
//! canned event streams: it hands on each event once, opening the stream
//! again from the last id when it ends; a malformed stream or a refusal
//! ends it, said; a busy server is tried again, for a bounded time; a
//! stream that keeps ending with no event is bounded too.
use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use quecto::application::project_board::ports::feed::{
    BoardRevision, BoardWatch, FeedEvent, WatchFlow,
};
use quecto::domain::project_board::value_objects::telemetry::BoardId;
use quecto::infrastructure::http::board_service::client::BoardServiceClient;
use quecto::infrastructure::http::board_watch::{RemoteBoardWatch, WatchTiming};

const BOARD: &str = "0123456789abcdef";

fn sse(body: &str) -> Vec<u8> {
    let mut reply = b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\
                      transfer-encoding: chunked\r\nconnection: close\r\n\r\n"
        .to_vec();
    if !body.is_empty() {
        reply.extend(format!("{:x}\r\n{body}\r\n", body.len()).as_bytes());
    }
    reply.extend(b"0\r\n\r\n");
    reply
}

fn busy() -> Vec<u8> {
    let body = r#"{"error":"too many change feeds open","code":"busy"}"#;
    format!(
        "HTTP/1.1 503 Service Unavailable\r\ncontent-type: application/json\r\n\
         content-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

fn epoch(id: u64, epoch: u64) -> String {
    format!("id: {id}\nevent: epoch\ndata: {{\"kind\":\"epoch\",\"epoch\":{epoch}}}\n\n")
}

fn revision(id: u64) -> String {
    format!(
        "id: {id}\nevent: revision\ndata: {{\"kind\":\"revision\",\"epoch\":1,\"revision\":{id}}}\n\n"
    )
}

/// Serves `replies` one connection each on a temp socket; sends each head
/// the watch sent. Past the last reply, nothing answers.
fn canned(
    replies: Vec<Vec<u8>>,
    busy_for: Duration,
) -> (tempfile::TempDir, RemoteBoardWatch, mpsc::Receiver<String>) {
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
    let timing = WatchTiming {
        idle: Duration::from_secs(5),
        pause: Duration::from_millis(10),
        busy_for,
    };
    let watch = RemoteBoardWatch::timed(
        BoardServiceClient::unix(&socket),
        BoardId::of_hex(BOARD).expect("a board id"),
        timing,
    );
    (dir, watch, heard)
}

/// Every event `watch` hands on from `after` until revision `last`.
fn watched(
    watch: &RemoteBoardWatch,
    after: Option<u64>,
    last: u64,
) -> (Vec<FeedEvent>, Result<(), String>) {
    let mut told = Vec::new();
    let watched = watch.watch(after, &mut |event| {
        told.push(event);
        match event {
            FeedEvent::Revision(BoardRevision { revision, .. }) if revision == last => {
                WatchFlow::Stop
            }
            _ => WatchFlow::Continue,
        }
    });
    (told, watched)
}

fn revisions(told: &[FeedEvent]) -> Vec<u64> {
    told.iter()
        .filter_map(|event| match event {
            FeedEvent::Revision(revision) => Some(revision.revision),
            _ => None,
        })
        .collect()
}

#[test]
fn an_ended_stream_is_opened_again_from_its_last_id_and_nothing_repeats() {
    let first = format!(
        "retry: 1000\n\n{}{}{}",
        epoch(0, 1),
        revision(1),
        revision(2)
    );
    let second = format!("{}{}", epoch(2, 1), revision(3));
    let (_dir, watch, heads) = canned(vec![sse(&first), sse(&second)], Duration::from_secs(1));
    let (told, watched) = watched(&watch, Some(0), 3);
    assert_eq!(watched, Ok(()));
    assert_eq!(revisions(&told), vec![1, 2, 3]);
    let first_head = heads.recv_timeout(Duration::from_secs(5)).expect("a head");
    assert!(first_head.starts_with(&format!("GET /v1/boards/{BOARD}/events HTTP/1.1\r\n")));
    assert!(first_head.contains("last-event-id: 0\r\n"), "{first_head}");
    let second_head = heads.recv_timeout(Duration::from_secs(5)).expect("a head");
    assert!(
        second_head.contains("last-event-id: 2\r\n"),
        "{second_head}"
    );
}

/// Review (#2757 L1): an epoch the watch has already handed on, told again
/// as a stream reopens, is not handed on again.
#[test]
fn an_unchanged_epoch_is_handed_on_once() {
    let first = format!("{}{}", epoch(0, 1), revision(1));
    let second = format!("{}{}{}", epoch(1, 1), epoch(1, 2), revision(2));
    let (_dir, watch, _) = canned(vec![sse(&first), sse(&second)], Duration::from_secs(1));
    let (told, watched) = watched(&watch, Some(0), 2);
    assert_eq!(watched, Ok(()));
    let epochs: Vec<u64> = told
        .iter()
        .filter_map(|event| match event {
            FeedEvent::Epoch { epoch } => Some(*epoch),
            _ => None,
        })
        .collect();
    assert_eq!(epochs, vec![1, 2], "{told:?}");
}

/// Review (#2757 L4): a reset is handed on, and the watch reads on from its
/// `after`.
#[test]
fn a_reset_is_handed_on_and_read_on_from() {
    let first = format!(
        "{}id: 1\nevent: reset\ndata: {{\"kind\":\"reset\",\"after\":1}}\n\n",
        revision(1)
    );
    let (_dir, watch, heads) = canned(vec![sse(&first), sse(&revision(2))], Duration::from_secs(1));
    let (told, watched) = watched(&watch, Some(0), 2);
    assert_eq!(watched, Ok(()));
    assert!(told.contains(&FeedEvent::Reset { after: 1 }), "{told:?}");
    assert_eq!(revisions(&told), vec![1, 2]);
    let _ = heads.recv_timeout(Duration::from_secs(5)).expect("a head");
    let second = heads.recv_timeout(Duration::from_secs(5)).expect("a head");
    assert!(second.contains("last-event-id: 1\r\n"), "{second}");
}

#[test]
fn a_refusal_ends_the_watch_saying_why() {
    let body = r#"{"error":"unauthorized","code":"token"}"#;
    let reply = format!(
        "HTTP/1.1 401 Unauthorized\r\ncontent-type: application/json\r\n\
         content-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let (_dir, watch, _) = canned(vec![reply.into_bytes()], Duration::from_secs(1));
    let watched = watch.watch(None, &mut |_| WatchFlow::Continue);
    let why = watched.expect_err("a refusal ends it");
    assert!(why.contains("401 token"), "{why}");
}

/// Review (#2757 M1): a malformed stream (here an unknown field) ends the
/// watch with its error, never a quiet reconnect.
#[test]
fn a_malformed_stream_ends_the_watch_saying_why() {
    let (_dir, watch, _) = canned(
        vec![sse("title: x\n\n"), sse(&revision(1))],
        Duration::from_secs(1),
    );
    let (told, watched) = watched(&watch, Some(0), 1);
    let why = watched.expect_err("a malformed stream ends it");
    assert!(why.contains("malformed"), "{why}");
    assert!(told.is_empty(), "{told:?}");
}

/// Review (#2757 M1): a server that answers 200 and closes with no event
/// is tried a bounded number of times, then the watch ends saying so.
#[test]
fn a_stream_that_keeps_ending_with_no_event_is_bounded() {
    let empty = (0..20).map(|_| sse("")).collect();
    let (_dir, watch, heads) = canned(empty, Duration::from_secs(1));
    let began = Instant::now();
    let watched = watch.watch(Some(0), &mut |_| WatchFlow::Continue);
    let why = watched.expect_err("bounded");
    assert!(why.contains("no event"), "{why}");
    assert!(began.elapsed() < Duration::from_secs(5));
    assert_eq!(heads.try_iter().count(), 5, "five opens, then it gives up");
}

/// Review (#2757 M2): a busy server is tried again, with backoff, and the
/// watch goes on when a place frees.
#[test]
fn a_busy_server_is_tried_again() {
    let (_dir, watch, heads) = canned(
        vec![busy(), busy(), busy(), sse(&revision(1))],
        Duration::from_secs(5),
    );
    let (told, watched) = watched(&watch, Some(0), 1);
    assert_eq!(watched, Ok(()));
    assert_eq!(revisions(&told), vec![1]);
    assert_eq!(heads.try_iter().count(), 4);
}

/// Review (#2757 M2): busy for longer than its bound, the watch ends,
/// saying the server stayed busy; busy answers count apart from failed
/// opens (more than 5 are tried).
#[test]
fn a_server_busy_past_its_bound_ends_the_watch() {
    let replies = (0..200).map(|_| busy()).collect();
    let (_dir, watch, heads) = canned(replies, Duration::from_millis(400));
    let began = Instant::now();
    let watched = watch.watch(Some(0), &mut |_| WatchFlow::Continue);
    let why = watched.expect_err("bounded");
    assert!(why.contains("busy"), "{why}");
    assert!(
        began.elapsed() >= Duration::from_millis(400),
        "{:?}",
        began.elapsed()
    );
    assert!(
        began.elapsed() < Duration::from_secs(3),
        "{:?}",
        began.elapsed()
    );
    assert!(
        heads.try_iter().count() > 5,
        "busy answers are counted apart"
    );
}
