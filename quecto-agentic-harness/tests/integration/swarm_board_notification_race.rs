//! Notification claims under contention (#2276,
//! `test_concurrent_notification_claims_do_not_duplicate_hints`): clients
//! acting as one member claim its hints at once, each through its own
//! board client on the one file. `_notifications` reads the events after
//! the member's cursor and advances it in one `BEGIN IMMEDIATE`
//! transaction, so the one hint is sent once in all, and a later claim
//! finds none, whether every client is a Rust board or Python and Rust
//! boards share the file.
//!
//! Contention is forced as in `swarm_board_claim_race`: a connection
//! outside every board holds the write lock until each contender has been
//! refused as locked at least once, so all of them retry against each
//! other when it goes.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use quecto::application::swarm::dto::BoardLocation;
use quecto::composition::swarm::build_swarm_board_handles;
use quecto::infrastructure::tools::swarm_board_dispatch::call;
use serde_json::{Value, json};
use serial_test::serial;

use crate::swarm_board_diff_runs::swarm_board_diff::Outcome;
use crate::swarm_board_diff_runs::swarm_board_diff::python::PyBoard;

const LOCKED: &str = "coordination store unavailable or contended: database is locked";
const RETRY_BOUND: Duration = Duration::from_secs(60);
const CLIENTS: usize = 4;

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

/// A running run with `worker` live, and one message from `worker` to
/// `parent`: the one hint `worker`'s claim sends.
fn board(dir: &std::path::Path) -> BoardLocation {
    let location = BoardLocation {
        database: dir.join("swarm.sqlite"),
        checkout: dir.to_path_buf(),
    };
    let parent = build_swarm_board_handles(location.clone(), None);
    call(
        &parent,
        "parent",
        "create_run",
        json!(["race", [], [{"id": "t", "kind": "command", "description": "d"}], 3, now() + 3_600.0]),
    )
    .unwrap();
    call(&parent, "parent", "_admit", json!(["worker", "w"])).unwrap();
    call(
        &parent,
        "parent",
        "_activate",
        json!(["worker", "w", 1, "t", null]),
    )
    .unwrap();
    call(
        &parent,
        "worker",
        "send",
        json!(["one", "parent", "Please review"]),
    )
    .unwrap();
    location
}

/// A writer outside every board, holding the file's write lock until every
/// contender has been refused as locked.
fn hold_lock_until(database: &std::path::Path, locked_out: Arc<AtomicUsize>) -> impl FnOnce() {
    let connection = rusqlite::Connection::open(database).unwrap();
    connection.execute_batch("BEGIN IMMEDIATE").unwrap();
    move || {
        let began = Instant::now();
        while locked_out.load(Ordering::SeqCst) < CLIENTS {
            assert!(
                began.elapsed() < RETRY_BOUND,
                "not every contender reached the lock"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        connection.execute_batch("COMMIT").unwrap();
    }
}

/// Claims `worker`'s hints until the answer is not the lock, counting the
/// contender as locked out once.
fn contend(
    locked_out: &AtomicUsize,
    mut claim: impl FnMut() -> Result<Value, String>,
) -> (Result<Value, String>, usize) {
    let began = Instant::now();
    let mut retries = 0;
    loop {
        match claim() {
            Err(text) if text == LOCKED => {
                if retries == 0 {
                    locked_out.fetch_add(1, Ordering::SeqCst);
                }
                retries += 1;
                assert!(began.elapsed() < RETRY_BOUND, "a claim stayed locked out");
            }
            definitive => return (definitive, retries),
        }
    }
}

/// One hint in all, to `parent`, and none left for a later claim.
fn judge(answers: &[(Result<Value, String>, usize)], location: &BoardLocation) {
    assert!(
        answers.iter().all(|(_, retries)| *retries >= 1),
        "every contender retried a locked board: {answers:?}"
    );
    let hints: Vec<&Value> = answers
        .iter()
        .map(|(answer, _)| answer.as_ref().expect("every claim answers"))
        .flat_map(|answer| answer.as_array().expect("a member list"))
        .collect();
    assert_eq!(hints.len(), 1, "the hint is sent once: {answers:?}");
    assert_eq!(hints[0]["id"], json!("parent"));
    let later = build_swarm_board_handles(location.clone(), None);
    assert_eq!(
        call(&later, "worker", "_notifications", json!([])).unwrap(),
        json!([])
    );
}

fn run_race(python_every: Option<usize>) {
    let dir = tempfile::tempdir().unwrap();
    let location = board(dir.path());
    let locked_out = Arc::new(AtomicUsize::new(0));
    let release = hold_lock_until(&location.database, locked_out.clone());
    let start = Arc::new(Barrier::new(CLIENTS));
    let threads: Vec<_> = (0..CLIENTS)
        .map(|index| {
            let location = location.clone();
            let workdir = dir.path().join(format!("python-{index}"));
            let start = start.clone();
            let locked_out = locked_out.clone();
            std::thread::spawn(move || {
                if python_every.is_some_and(|every| index % every == 0) {
                    std::fs::create_dir_all(&workdir).unwrap();
                    let mut board =
                        PyBoard::start(&location.database, &location.checkout, &workdir);
                    start.wait();
                    contend(&locked_out, || {
                        match board.call("worker", "_notifications", "[]", now()) {
                            Outcome::Ok(value) => Ok(value),
                            Outcome::Refused(text) => Err(text),
                            Outcome::Raised(raised) => panic!("Python raised {raised}"),
                        }
                    })
                } else {
                    let handles = build_swarm_board_handles(location, None);
                    start.wait();
                    contend(&locked_out, || {
                        call(&handles, "worker", "_notifications", json!([]))
                            .map_err(|refusal| refusal.message().to_owned())
                    })
                }
            })
        })
        .collect();
    release();
    let answers: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    judge(&answers, &location);
}

#[test]
#[serial]
fn concurrent_notification_claims_do_not_duplicate_hints() {
    run_race(None);
}

/// Two Python clients and two Rust clients claim at once.
#[test]
#[serial]
fn concurrent_python_and_rust_notification_claims_do_not_duplicate_hints() {
    run_race(Some(2));
}
