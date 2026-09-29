//! #2303: the lock a board transaction waits for is measured where it is
//! taken, only while a call is metered, and waiting behaves exactly as
//! SQLite's own 500 ms busy timeout does.
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use rusqlite::Connection;

use super::{CallMeter, active, busy_delay, metered};
use crate::infrastructure::persistence::swarm_board::store::{BUSY_TIMEOUT, BoardStore};

fn created() -> (tempfile::TempDir, BoardStore) {
    let dir = tempfile::TempDir::new().unwrap();
    let store = BoardStore::new(dir.path().join("swarm.sqlite"));
    store.transaction(true, |_| Ok(())).unwrap();
    (dir, store)
}

/// A second connection holds `BEGIN IMMEDIATE` on `store`'s file for
/// `hold`, then commits. Returns once the lock is held.
fn hold(store: &BoardStore, hold: Duration) -> thread::JoinHandle<()> {
    let path = store.path().to_path_buf();
    let (held, taken) = mpsc::channel();
    let holder = thread::spawn(move || {
        let connection = Connection::open(path).unwrap();
        connection.execute_batch("BEGIN IMMEDIATE").unwrap();
        held.send(()).unwrap();
        thread::sleep(hold);
        connection.execute_batch("COMMIT").unwrap();
    });
    taken.recv_timeout(Duration::from_secs(10)).unwrap();
    holder
}

#[test]
fn a_held_lock_is_measured_as_a_busy_wait() {
    let (_dir, store) = created();
    let holder = hold(&store, Duration::from_millis(100));
    let (outcome, meter) = metered(|| store.transaction(false, |_| Ok(())));
    holder.join().unwrap();
    outcome.unwrap();
    assert!(meter.busy, "{meter:?}");
    assert!(meter.lock_wait >= Duration::from_millis(100), "{meter:?}");
    assert!(meter.lock_wait < BUSY_TIMEOUT, "{meter:?}");
}

#[test]
fn a_free_lock_is_not_busy_and_the_waits_of_a_call_add_up() {
    let (_dir, store) = created();
    assert!(!active(), "no call is metered outside `metered`");
    let (inside, meter) = metered(|| {
        store.transaction(false, |_| Ok(())).unwrap();
        store.transaction(false, |_| Ok(())).unwrap();
        active()
    });
    assert!(inside, "a call is metered inside `metered`");
    assert!(!active(), "the scope ends with the call");
    assert!(!meter.busy, "{meter:?}");
    assert!(meter.lock_wait < Duration::from_millis(100), "{meter:?}");
    assert_eq!(meter.run_id, None, "the store alone reads no run");
}

/// A lock held past the timeout is refused with the text an unmetered
/// transaction gives, after the same wait.
#[test]
fn a_metered_transaction_gives_up_as_sqlites_busy_timeout_does() {
    let (_dir, store) = created();
    let holder = hold(&store, Duration::from_millis(1_500));
    let started = Instant::now();
    let (outcome, meter) = metered(|| store.transaction(false, |_| Ok(())));
    let waited = started.elapsed();
    let plain = store.transaction(false, |_| Ok(()));
    holder.join().unwrap();
    assert_eq!(outcome, plain, "the same refusal, metered or not");
    assert_eq!(
        outcome.unwrap_err().0,
        "coordination store unavailable or contended: database is locked"
    );
    assert!(meter.busy, "{meter:?}");
    assert!(waited >= BUSY_TIMEOUT, "{waited:?}");
    assert!(waited < Duration::from_millis(1_400), "{waited:?}");
}

/// SQLite's `sqliteDefaultBusyCallback` schedule under a 500 ms timeout:
/// 1, 2, 5, 10, 15, 20, 25, 25, 25, 50, 50, 100 ms, then 100 ms each,
/// the last wait cut so the total is exactly the timeout.
#[test]
fn the_busy_schedule_is_sqlites_default_under_the_timeout() {
    let delays: Vec<u64> = (0..)
        .map_while(busy_delay)
        .map(|delay| u64::try_from(delay.as_millis()).unwrap())
        .collect();
    assert_eq!(
        delays,
        [1, 2, 5, 10, 15, 20, 25, 25, 25, 50, 50, 100, 100, 72]
    );
    assert_eq!(
        delays.iter().sum::<u64>(),
        u64::try_from(BUSY_TIMEOUT.as_millis()).unwrap()
    );
}

/// Metering scopes nest: an inner call's measure is its own, and the
/// outer call's resumes after it.
#[test]
fn nested_scopes_keep_their_own_measure() {
    let (_dir, store) = created();
    let ((_, inner), outer) = metered(|| {
        let inner = metered(|| store.transaction(false, |_| Ok(())).unwrap());
        assert!(active(), "the outer scope is restored");
        inner
    });
    assert_eq!(inner.busy, outer.busy);
    assert_ne!(
        inner,
        CallMeter::default(),
        "the inner call measured its lock"
    );
    assert!(!active());
}
