//! Claim atomicity under contention (#2272): six members claim one task at
//! once, each through its own board client on the one file. The claim
//! reads the task and writes the owner and token in one `BEGIN IMMEDIATE`
//! transaction, so exactly one wins and every other is refused `task is
//! not ready to claim`. (Until #2283 half the clients were Python boards;
//! the board's answers are Python's by the golden fixtures, so the race is
//! Rust boards alone.)
//!
//! Contention is forced as in `swarm_board_admission_race`: a connection
//! outside every board holds the write lock until each contender has been
//! refused as locked at least once, so all of them retry against each
//! other when it goes.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use quecto::application::swarm::dto::BoardLocation;
use quecto::composition::swarm::{SwarmBoardHandles, build_swarm_board_handles};
use quecto::infrastructure::tools::swarm_board_dispatch::call;
use serde_json::{Value, json};
use serial_test::serial;

const LOCKED: &str = "coordination store unavailable or contended: database is locked";
const NOT_READY: &str = "task is not ready to claim";
const RETRY_BOUND: Duration = Duration::from_secs(60);
const MEMBERS: usize = 6;

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

/// A running run with `MEMBERS` live members and one ready task.
fn board(dir: &std::path::Path) -> BoardLocation {
    let location = BoardLocation {
        database: dir.join("swarm.sqlite"),
        checkout: dir.to_path_buf(),
    };
    let parent = build_swarm_board_handles(location.clone(), None);
    let limit = i64::try_from(MEMBERS).unwrap() + 1;
    call(
        &parent,
        "parent",
        "create_run",
        json!(["race", [], [{"id": "t", "kind": "command", "description": "d"}], limit, now() + 3_600.0]),
    )
    .unwrap();
    for index in 0..MEMBERS {
        let member = format!("member-{index}");
        call(&parent, "parent", "_admit", json!([member, member])).unwrap();
        call(
            &parent,
            "parent",
            "_activate",
            json!([member, member, index, "t", null]),
        )
        .unwrap();
    }
    call(
        &parent,
        "parent",
        "task_create",
        json!(["r", "contested", ["one owner"]]),
    )
    .unwrap();
    location
}

/// A writer outside every board, holding the file's write lock until every
/// contender has been refused as locked: the returned closure waits for
/// that and commits.
fn hold_lock_until(database: &std::path::Path, locked_out: Arc<AtomicUsize>) -> impl FnOnce() {
    let connection = rusqlite::Connection::open(database).unwrap();
    connection.execute_batch("BEGIN IMMEDIATE").unwrap();
    move || {
        let began = Instant::now();
        while locked_out.load(Ordering::SeqCst) < MEMBERS {
            assert!(
                began.elapsed() < RETRY_BOUND,
                "not every contender reached the lock"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        connection.execute_batch("COMMIT").unwrap();
    }
}

/// Claims task 1 as `member` until the answer is not the lock, counting
/// the contender as locked out once.
fn contend(
    member: &str,
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
                assert!(began.elapsed() < RETRY_BOUND, "{member} stayed locked out");
            }
            definitive => return (definitive, retries),
        }
    }
}

fn rust_claim(handles: &SwarmBoardHandles, member: &str) -> Result<Value, String> {
    call(handles, member, "claim", json!([1])).map_err(|refusal| refusal.message().to_owned())
}

fn judge(answers: &[(Result<Value, String>, usize)], location: &BoardLocation) {
    assert!(
        answers.iter().all(|(_, retries)| *retries >= 1),
        "every contender retried a locked board: {answers:?}"
    );
    let winners: Vec<&Value> = answers
        .iter()
        .filter_map(|(answer, _)| answer.as_ref().ok())
        .collect();
    assert_eq!(winners.len(), 1, "exactly one claim wins: {answers:?}");
    assert!(
        answers
            .iter()
            .filter_map(|(answer, _)| answer.as_ref().err())
            .all(|refusal| refusal == NOT_READY),
        "{answers:?}"
    );
    let connection = rusqlite::Connection::open(&location.database).unwrap();
    let (owner, token, claimed): (String, String, i64) = connection
        .query_row(
            "SELECT owner, token, (SELECT count(*) FROM events WHERE action='claimed') FROM tasks WHERE id=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(json!(owner), winners[0]["owner"]);
    assert_eq!(json!(token), winners[0]["token"]);
    assert_eq!(claimed, 1, "one claimed event");
}

#[test]
#[serial]
fn concurrent_claims_of_one_task_have_exactly_one_winner() {
    let dir = tempfile::tempdir().unwrap();
    let location = board(dir.path());
    let locked_out = Arc::new(AtomicUsize::new(0));
    let release = hold_lock_until(&location.database, locked_out.clone());
    let start = Arc::new(Barrier::new(MEMBERS));
    let threads: Vec<_> = (0..MEMBERS)
        .map(|index| {
            let location = location.clone();
            let start = start.clone();
            let locked_out = locked_out.clone();
            std::thread::spawn(move || {
                let handles = build_swarm_board_handles(location, None);
                let member = format!("member-{index}");
                start.wait();
                contend(&member, &locked_out, || rust_claim(&handles, &member))
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
