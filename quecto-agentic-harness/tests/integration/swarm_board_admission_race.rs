//! Launch capacity under contention (#2271): many harnesses admit members
//! into one board file at once. Admission reads the usage and inserts the
//! row in one `BEGIN IMMEDIATE` transaction, so no interleaving admits past
//! the member limit (round-1 review N2; until #2283 Python boards raced
//! beside the Rust ones: the board's answers are Python's by the golden
//! fixtures, so the race is Rust boards alone).
//!
//! Contention is forced, not hoped for (round-2 review): a separate
//! connection holds `BEGIN IMMEDIATE` while every contender starts, and
//! releases it only once each contender has been refused as locked at
//! least once (each board waits out the 500 ms busy timeout, then refuses).
//! Every contender is then retrying against the others when the lock goes.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use quecto::application::swarm::dto::BoardLocation;
use quecto::application::swarm::ports::BoardRepository;
use quecto::composition::swarm::build_swarm_board_handles;
use quecto::infrastructure::persistence::swarm_board::repository::SqliteBoardRepository;
use quecto::infrastructure::tools::swarm_board_dispatch::call;
use serde_json::json;
use serial_test::serial;

const LOCKED: &str = "coordination store unavailable or contended: database is locked";
const LIMIT_REACHED: &str = "swarm limit 4, current usage 4; reuse the existing pool";
/// How long one thread may keep retrying a locked board before the test
/// is presumed stuck.
const RETRY_BOUND: Duration = Duration::from_secs(60);

/// A writer outside every board, holding the file's write lock.
struct LockHolder(rusqlite::Connection);

impl LockHolder {
    fn take(database: &std::path::Path) -> Self {
        let connection = rusqlite::Connection::open(database).unwrap();
        connection.execute_batch("BEGIN IMMEDIATE").unwrap();
        Self(connection)
    }

    /// Releases the lock once `locked_out` contenders have each been
    /// refused as locked, so every one of them contends at release.
    fn release_after(self, locked_out: &AtomicUsize, contenders: usize) {
        let began = Instant::now();
        while locked_out.load(Ordering::SeqCst) < contenders {
            assert!(
                began.elapsed() < RETRY_BOUND,
                "only {} of {contenders} contenders reached the held lock",
                locked_out.load(Ordering::SeqCst)
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        self.0.execute_batch("COMMIT").unwrap();
    }
}

/// One contender's lock retries: the first also counts the contender as
/// locked out.
#[derive(Default)]
struct Retries(usize);

impl Retries {
    fn locked(&mut self, locked_out: &AtomicUsize, began: Instant, who: &str) {
        if self.0 == 0 {
            locked_out.fetch_add(1, Ordering::SeqCst);
        }
        self.0 += 1;
        assert!(began.elapsed() < RETRY_BOUND, "{who} stayed locked out");
    }
}

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
    let parent = build_swarm_board_handles(location.clone(), None);
    call(
        &parent,
        "parent",
        "create_run",
        json!(["race", [], [{"id": "t", "kind": "command", "description": "d"}], 4, now + 3_600.0]),
    )
    .unwrap();
    let holder = LockHolder::take(&location.database);
    let locked_out = Arc::new(AtomicUsize::new(0));
    let start = Arc::new(Barrier::new(8));
    let threads: Vec<_> = (0..8)
        .map(|index| {
            let location = location.clone();
            let start = start.clone();
            let locked_out = locked_out.clone();
            std::thread::spawn(move || {
                // Each thread is its own harness: its own handles and store.
                let handles = build_swarm_board_handles(location, None);
                start.wait();
                let began = Instant::now();
                let mut retries = Retries::default();
                loop {
                    let answer = call(
                        &handles,
                        "parent",
                        "_admit",
                        json!([format!("child-{index}"), format!("reserve-{index}")]),
                    );
                    match answer {
                        Err(refusal) if refusal.message() == LOCKED => {
                            retries.locked(&locked_out, began, &format!("child-{index}"));
                        }
                        definitive => return (definitive, retries.0),
                    }
                }
            })
        })
        .collect();
    holder.release_after(&locked_out, 8);
    let (answers, retries): (Vec<_>, Vec<usize>) = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .unzip();
    assert!(
        retries.iter().all(|&count| count >= 1),
        "every contender retried a locked board: {retries:?}"
    );
    let admitted = answers.iter().filter(|answer| answer.is_ok()).count();
    let refused: Vec<_> = answers
        .iter()
        .filter_map(|answer| answer.as_ref().err())
        .collect();
    assert_eq!(admitted, 3, "4 minus the coordinator: {answers:?}");
    assert_eq!(refused.len(), 5, "{answers:?}");
    assert!(
        refused
            .iter()
            .all(|refusal| refusal.message() == LIMIT_REACHED),
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
    let integrity: String = rusqlite::Connection::open(&location.database)
        .unwrap()
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
}
