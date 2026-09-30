//! Notification claims under contention (#2276,
//! `test_concurrent_notification_claims_do_not_duplicate_hints`): clients
//! acting as one member claim its hints at once, each through its own
//! board client on the one file. `_notifications` reads the events after
//! the member's cursor and advances it in one `BEGIN IMMEDIATE`
//! transaction, so the one hint is sent once in all, and a later claim
//! finds none. (Until #2283 half the clients were Python boards; the
//! board's answers are Python's by the golden fixtures, so the races are
//! Rust boards alone.)
//!
//! `_accept_wake` is raced too, as each member's first claim on a file
//! with no `wake_cursors` table: the claim creates the table inside its
//! `BEGIN IMMEDIATE` transaction, so it is created once, and per member
//! one claim moves the cursor and is woken.
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

/// Every contender's answer to `method` with `arguments`, as the member
/// `member(index)` names, each its own Rust board.
fn race(
    location: &BoardLocation,
    member: fn(usize) -> &'static str,
    (method, arguments): (&'static str, Value),
) -> Vec<(Result<Value, String>, usize)> {
    let locked_out = Arc::new(AtomicUsize::new(0));
    let release = hold_lock_until(&location.database, locked_out.clone());
    let start = Arc::new(Barrier::new(CLIENTS));
    let threads: Vec<_> = (0..CLIENTS)
        .map(|index| {
            let location = location.clone();
            let (start, locked_out, arguments) =
                (start.clone(), locked_out.clone(), arguments.clone());
            std::thread::spawn(move || {
                let handles = build_swarm_board_handles(location, None);
                start.wait();
                contend(&locked_out, || {
                    call(&handles, member(index), method, arguments.clone())
                        .map_err(|refusal| refusal.message().to_owned())
                })
            })
        })
        .collect();
    release();
    let answers: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    assert!(
        answers.iter().all(|(_, retries)| *retries >= 1),
        "every contender retried a locked board: {answers:?}"
    );
    answers
}

#[test]
#[serial]
fn concurrent_notification_claims_do_not_duplicate_hints() {
    let dir = tempfile::tempdir().unwrap();
    let location = board(dir.path());
    let answers = race(&location, |_| "worker", ("_notifications", json!([])));
    judge(&answers, &location);
}

/// [`board`], with a message from `parent` to `worker` too, so each has an
/// unread message waking it; no wake claim has run, so the file holds no
/// `wake_cursors` table. Answers the board's generation.
fn fresh_wake_board(dir: &std::path::Path) -> (BoardLocation, i64) {
    let location = board(dir);
    let parent = build_swarm_board_handles(location.clone(), None);
    call(
        &parent,
        "parent",
        "send",
        json!(["two", "worker", "Please fix"]),
    )
    .unwrap();
    let connection = rusqlite::Connection::open(&location.database).unwrap();
    assert_eq!(
        wake_tables(&connection),
        0,
        "a fresh file has no wake cursors"
    );
    let generation = connection
        .query_row("SELECT max(id) FROM events", [], |row| row.get(0))
        .unwrap();
    (location, generation)
}

fn wake_tables(connection: &rusqlite::Connection) -> i64 {
    connection
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='wake_cursors'",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

/// The first two contenders claim as `parent`, the others as `worker`.
fn claimant(index: usize) -> &'static str {
    if index < CLIENTS / 2 {
        "parent"
    } else {
        "worker"
    }
}

/// Each member's first claim of the generation, contended on a file with
/// no `wake_cursors` table: whichever contender creates the table, the
/// others re-prepare after it. Per member exactly one claim moves the
/// cursor and is woken, and the other answers `false`; the table exists
/// once, the file is intact, each member's cursor is at the generation,
/// and a later claim is not woken.
#[test]
#[serial]
fn concurrent_first_wake_claims_create_the_cursors_once() {
    let dir = tempfile::tempdir().unwrap();
    let (location, generation) = fresh_wake_board(dir.path());
    let answers = race(&location, claimant, ("_accept_wake", json!([generation])));
    for member in ["parent", "worker"] {
        let mine: Vec<&Value> = answers
            .iter()
            .enumerate()
            .filter(|(index, _)| claimant(*index) == member)
            .map(|(_, (answer, _))| answer.as_ref().expect("every claim answers"))
            .collect();
        assert_eq!(mine.len(), CLIENTS / 2, "{member}: {answers:?}");
        let woken = mine
            .iter()
            .filter(|answer| ***answer == json!(true))
            .count();
        let quiet = mine
            .iter()
            .filter(|answer| ***answer == json!(false))
            .count();
        assert_eq!(
            (woken, quiet),
            (1, CLIENTS / 2 - 1),
            "{member}: {answers:?}"
        );
    }
    let connection = rusqlite::Connection::open(&location.database).unwrap();
    assert_eq!(wake_tables(&connection), 1);
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
    let cursors: Vec<(String, i64)> = connection
        .prepare("SELECT actor,event FROM wake_cursors ORDER BY actor")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        cursors,
        [
            ("parent".to_owned(), generation),
            ("worker".to_owned(), generation)
        ]
    );
    let later = build_swarm_board_handles(location, None);
    for member in ["parent", "worker"] {
        assert_eq!(
            call(&later, member, "_accept_wake", json!([generation])).unwrap(),
            json!(false),
            "{member}"
        );
    }
}
