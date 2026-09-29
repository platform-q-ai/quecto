//! Launch capacity under contention (#2271): many harnesses admit members
//! into one board file at once. Admission reads the usage and inserts the
//! row in one `BEGIN IMMEDIATE` transaction, so no interleaving admits past
//! the member limit, whether every writer is a Rust board or Python and
//! Rust boards share the file (round-1 review N2).
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use quecto::application::swarm::dto::BoardLocation;
use quecto::application::swarm::ports::BoardRepository;
use quecto::composition::swarm::build_swarm_board_handles;
use quecto::domain::swarm::BoardError;
use quecto::infrastructure::persistence::swarm_board::repository::SqliteBoardRepository;
use quecto::infrastructure::tools::swarm_board_dispatch::call;
use serde_json::json;
use serial_test::serial;

use crate::swarm_board_diff_runs::swarm_board_diff::Outcome;
use crate::swarm_board_diff_runs::swarm_board_diff::python::PyBoard;

const LOCKED: &str = "coordination store unavailable or contended: database is locked";
const LIMIT_REACHED: &str = "swarm limit 4, current usage 4; reuse the existing pool";
/// How long one thread may keep retrying a locked board before the test
/// is presumed stuck.
const RETRY_BOUND: Duration = Duration::from_secs(60);

#[test]
#[serial]
fn concurrent_rust_admissions_never_exceed_the_limit() {
    let dir = tempfile::tempdir().unwrap();
    let location = BoardLocation {
        database: dir.path().join("swarm.sqlite"),
        checkout: dir.path().to_path_buf(),
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    let parent = build_swarm_board_handles(location.clone());
    call(
        &parent,
        "parent",
        "create_run",
        json!(["race", [], [{"id": "t", "kind": "command", "description": "d"}], 4, now + 3_600.0]),
    )
    .unwrap();
    let start = Arc::new(Barrier::new(8));
    let threads: Vec<_> = (0..8)
        .map(|index| {
            let location = location.clone();
            let start = start.clone();
            std::thread::spawn(move || {
                // Each thread is its own harness: its own handles and store.
                let handles = build_swarm_board_handles(location);
                start.wait();
                let began = Instant::now();
                loop {
                    let answer = call(
                        &handles,
                        "parent",
                        "_admit",
                        json!([format!("child-{index}"), format!("reserve-{index}")]),
                    );
                    match answer {
                        Err(BoardError(text)) if text == LOCKED => {
                            assert!(
                                began.elapsed() < RETRY_BOUND,
                                "child-{index} stayed locked out"
                            );
                        }
                        definitive => return definitive,
                    }
                }
            })
        })
        .collect();
    let answers: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    let admitted = answers.iter().filter(|answer| answer.is_ok()).count();
    let refused: Vec<_> = answers
        .iter()
        .filter_map(|answer| answer.as_ref().err())
        .collect();
    assert_eq!(admitted, 3, "4 minus the coordinator: {answers:?}");
    assert_eq!(refused.len(), 5, "{answers:?}");
    assert!(
        refused.iter().all(|refusal| refusal.0 == LIMIT_REACHED),
        "{answers:?}"
    );
    let repository = SqliteBoardRepository::new(&location);
    let mut usage = None;
    repository
        .atomic(false, &mut |transaction| {
            usage = Some(transaction.usage()?);
            Ok(())
        })
        .unwrap();
    assert_eq!(usage, Some(4));
}

/// Python boards and Rust boards admit into one file at once: four
/// `python3` children running the board's `.py` sources and four Rust
/// harnesses race eight members for five places. Each retries only a
/// locked store, as a harness would; the rest is decided by the one
/// transaction both implementations admit in.
#[test]
#[serial]
fn concurrent_python_and_rust_admissions_never_exceed_the_limit() {
    const LIMIT: i64 = 6;
    let dir = tempfile::tempdir().unwrap();
    let location = BoardLocation {
        database: dir.path().join("swarm.sqlite"),
        checkout: dir.path().to_path_buf(),
    };
    let now = || {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs_f64()
    };
    let parent = build_swarm_board_handles(location.clone());
    call(
        &parent,
        "parent",
        "create_run",
        json!(["race", [], [{"id": "t", "kind": "command", "description": "d"}], LIMIT, now() + 3_600.0]),
    )
    .unwrap();
    let start = Arc::new(Barrier::new(8));
    let python: Vec<_> = (0..4)
        .map(|index| {
            let location = location.clone();
            let workdir = dir.path().join(format!("python-{index}"));
            let start = start.clone();
            std::thread::spawn(move || {
                std::fs::create_dir_all(&workdir).unwrap();
                let mut board = PyBoard::start(&location.database, &location.checkout, &workdir);
                let args =
                    json!([format!("py-{index}"), format!("reserve-py-{index}")]).to_string();
                start.wait();
                let began = Instant::now();
                loop {
                    match board.call("parent", "_admit", &args, now()) {
                        Outcome::Refused(text) if text == LOCKED => {
                            assert!(
                                began.elapsed() < RETRY_BOUND,
                                "py-{index} stayed locked out"
                            );
                        }
                        Outcome::Ok(_) => return Ok(()),
                        Outcome::Refused(text) => return Err(text),
                        Outcome::Raised(raised) => panic!("py-{index}: Python raised {raised}"),
                    }
                }
            })
        })
        .collect();
    let rust: Vec<_> = (0..4)
        .map(|index| {
            let location = location.clone();
            let start = start.clone();
            std::thread::spawn(move || {
                let handles = build_swarm_board_handles(location);
                start.wait();
                let began = Instant::now();
                loop {
                    let answer = call(
                        &handles,
                        "parent",
                        "_admit",
                        json!([format!("rs-{index}"), format!("reserve-rs-{index}")]),
                    );
                    match answer {
                        Err(BoardError(text)) if text == LOCKED => {
                            assert!(
                                began.elapsed() < RETRY_BOUND,
                                "rs-{index} stayed locked out"
                            );
                        }
                        Ok(_) => return Ok(()),
                        Err(BoardError(text)) => return Err(text),
                    }
                }
            })
        })
        .collect();
    let answers: Vec<Result<(), String>> = python
        .into_iter()
        .chain(rust)
        .map(|thread| thread.join().unwrap())
        .collect();
    let admitted = answers.iter().filter(|answer| answer.is_ok()).count();
    assert_eq!(
        i64::try_from(admitted).unwrap(),
        LIMIT - 1,
        "the limit minus the coordinator: {answers:?}"
    );
    let limit_reached =
        format!("swarm limit {LIMIT}, current usage {LIMIT}; reuse the existing pool");
    assert!(
        answers
            .iter()
            .filter_map(|answer| answer.as_ref().err())
            .all(|refusal| *refusal == limit_reached),
        "{answers:?}"
    );
    let connection = rusqlite::Connection::open(&location.database).unwrap();
    let usage: i64 = connection
        .query_row(
            "SELECT count(*) FROM members WHERE status IN ('live','reserved')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(usage <= LIMIT, "{usage} admitted past the limit {LIMIT}");
    assert_eq!(usage, LIMIT);
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
}
