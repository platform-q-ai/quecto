//! The change feed end to end (epic #2705 B10): a real server on a temp
//! socket over its SQLite board, written through its op route and watched
//! through the client's reader. Acceptance: feed p95 at most 100 ms.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::tests::{Synced, environment, seeded};
use super::*;
use crate::application::project_board::ports::feed::{BoardWatch, FeedEvent, WatchFlow};
use crate::infrastructure::http::board_service::client::{BoardServiceClient, READ_REPLY};
use crate::infrastructure::http::board_service::events::Opened;
use crate::infrastructure::http::board_service::limits::Method;
use crate::infrastructure::http::board_watch::{RemoteBoardWatch, decoded};
use crate::interface::cli::board_server::serve::{Started, serve};

/// A server serving `handles` on a temp socket until the sender is used.
struct Served {
    _dir: tempfile::TempDir,
    client: BoardServiceClient,
    board: BoardId,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<Result<(), String>>>,
}

impl Drop for Served {
    fn drop(&mut self) {
        let _ = self.stop.take().map(|stop| stop.send(()));
        let _ = self.thread.take().map(std::thread::JoinHandle::join);
    }
}

fn served(handles: BoardServerHandles) -> Served {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("board").join("board.sock");
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let (ready, started) = mpsc::channel();
    let at = socket.clone();
    let thread = std::thread::spawn(move || {
        serve(
            handles,
            &at,
            &[],
            Instant::now(),
            || async {
                let _ = stopped.await;
            },
            |started: &Started| ready.send(started.board.clone()).unwrap(),
        )
    });
    let board = started.recv_timeout(Duration::from_secs(10)).unwrap();
    Served {
        _dir: dir,
        client: BoardServiceClient::unix(&socket),
        board,
        stop: Some(stop),
        thread: Some(thread),
    }
}

/// Runs `request` on the server as a write: its answer.
fn write(served: &Served, request: Value) -> Value {
    let body = json!({"request": request, "front_end": "cli", "summary": false,
        "noted": false, "write_id": uuid::Uuid::new_v4().to_string(),
        "committer": {"name": "Ada", "email": "ada@example.com"}});
    let path = format!("/v1/boards/{}/op", served.board.as_str());
    let reply = served
        .client
        .send(Method::Post, &path, body.to_string().as_bytes(), READ_REPLY)
        .unwrap();
    assert_eq!(
        reply.status,
        200,
        "{}",
        String::from_utf8_lossy(&reply.body)
    );
    let answer: Value = serde_json::from_slice(&reply.body).unwrap();
    assert_eq!(answer["answer"]["ok"], json!(true), "{answer}");
    answer
}

/// The board's newest revision, from `/v1/status`.
fn tip(served: &Served) -> u64 {
    let reply = served
        .client
        .send(Method::Get, "/v1/status", b"", READ_REPLY)
        .unwrap();
    let status: Value = serde_json::from_slice(&reply.body).unwrap();
    let head = status["head"].as_str().unwrap();
    head.split_once(':').unwrap().1.parse().unwrap()
}

/// The revisions a watch saw, each with when it came.
type Revisions = mpsc::Receiver<(u64, Instant)>;
/// A watch's thread, and how its watch ended.
type Watcher = std::thread::JoinHandle<Result<(), String>>;

/// A watch on its own thread from `after`, sending each revision and when
/// it came, until `stop` is set (checked at each event).
fn watching(
    served: &Served,
    after: Option<u64>,
    stop: std::sync::Arc<AtomicBool>,
) -> (Revisions, Watcher) {
    let watch = RemoteBoardWatch::timed(
        served.client.clone(),
        served.board.clone(),
        Duration::from_secs(20),
        Duration::from_millis(50),
    );
    let (seen, revisions) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        watch.watch(after, &mut |event| {
            if let FeedEvent::Revision(revision) = event {
                let _ = seen.send((revision.revision, Instant::now()));
            }
            match stop.load(Ordering::SeqCst) {
                true => WatchFlow::Stop,
                false => WatchFlow::Continue,
            }
        })
    });
    (revisions, thread)
}

/// Waits for revision `want`: when it came; every revision before it is
/// pushed onto `seen` too.
fn until(revisions: &mpsc::Receiver<(u64, Instant)>, want: u64, seen: &mut Vec<u64>) -> Instant {
    loop {
        let (revision, at) = revisions
            .recv_timeout(Duration::from_secs(10))
            .unwrap_or_else(|_| panic!("revision {want} never came; saw {seen:?}"));
        seen.push(revision);
        if revision == want {
            return at;
        }
    }
}

/// Red tests (plan B10), end to end: a resumed watch is gap-free with
/// each revision once, no event carries the task's text, and the feed's
/// p95 from a write sent to its event received is at most 100 ms.
#[test]
fn the_feed_is_gap_free_textless_and_fast_end_to_end() {
    let (dir, work) = seeded();
    let handles =
        build_board_server_with(&environment(&dir, &work, "origin"), Arc::new(Synced)).unwrap();
    let served = served(handles);
    write(&served, json!({"op": "bootstrap"}));
    let stop = Arc::new(AtomicBool::new(false));
    let (revisions, watcher) = watching(&served, Some(0), Arc::clone(&stop));
    let mut seen = Vec::new();
    until(&revisions, tip(&served), &mut seen);
    let mut latencies = Vec::new();
    for at in 0..40 {
        let sent = Instant::now();
        write(
            &served,
            json!({"op": "create", "task": format!("t{at}"),
            "title": format!("SECRET-TITLE-{at}"), "status": "ready"}),
        );
        let came = until(&revisions, tip(&served), &mut seen);
        latencies.push(came.saturating_duration_since(sent));
    }
    latencies.sort_unstable();
    let p95 = latencies[latencies.len() * 95 / 100 - 1];
    eprintln!(
        "board feed: p50 {:?}, p95 {p95:?}, max {:?}",
        latencies[latencies.len() / 2],
        latencies.last().unwrap()
    );
    assert!(
        p95 <= Duration::from_millis(100),
        "feed p95 {p95:?}: {latencies:?}"
    );
    // The watch is cut; more lands while no one watches; it resumes.
    stop.store(true, Ordering::SeqCst);
    write(
        &served,
        json!({"op": "create", "task": "cut", "title": "Cut", "status": "ready"}),
    );
    watcher.join().unwrap().unwrap();
    while let Ok((revision, _)) = revisions.try_recv() {
        seen.push(revision);
    }
    let last = *seen.last().unwrap();
    for at in 0..5 {
        write(
            &served,
            json!({"op": "create", "task": format!("r{at}"), "title": "R", "status": "ready"}),
        );
    }
    let (resumed, thread) = watching(&served, Some(last), Arc::new(AtomicBool::new(false)));
    until(&resumed, tip(&served), &mut seen);
    assert_eq!(
        seen,
        (1..=tip(&served)).collect::<Vec<_>>(),
        "every revision once, in order"
    );
    drop(resumed);
    // The raw stream from the start: every event of the schema, no text.
    let path = format!("/v1/boards/{}/events", served.board.as_str());
    let Ok(Opened::Stream(mut reader)) =
        served
            .client
            .open_events(&path, Some(0), Duration::from_secs(5))
    else {
        panic!("the stream opens");
    };
    let mut told = 0;
    while told < tip(&served) {
        let event = reader.next_event().unwrap().unwrap();
        assert!(!format!("{event:?}").contains("SECRET"), "{event:?}");
        let (_, event) = decoded(&event).unwrap_or_else(|| panic!("off the schema: {event:?}"));
        if let FeedEvent::Revision(_) = event {
            told += 1;
        }
    }
    drop(reader);
    drop(served);
    // The resumed watch ends as its server goes.
    let _ = thread.join();
}
