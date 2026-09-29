//! #2303: the lock a board transaction waits for is measured where it is
//! taken, only while a call is metered, and waiting behaves exactly as
//! SQLite's own 500 ms busy timeout does.
//!
//! The lock tests do not race a timer against the call (#2303 review L5):
//! the holder takes the lock before the call starts and keeps it until it
//! is told to let go, so the call is certain to find it held; the margins
//! are wide enough for a loaded 2-vCPU runner.
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use rusqlite::Connection;

use super::{SqliteBoardCallMeter, active, busy_delay, metered};
use crate::application::swarm::ports::{BoardCallMeter, CallMeasure};
use crate::infrastructure::persistence::swarm_board::store::{BUSY_TIMEOUT, BoardStore};

fn created() -> (tempfile::TempDir, BoardStore) {
    let dir = tempfile::TempDir::new().unwrap();
    let store = BoardStore::new(dir.path().join("swarm.sqlite"));
    store.transaction(true, |_| Ok(())).unwrap();
    (dir, store)
}

/// A second connection holding `BEGIN IMMEDIATE` on a board file until
/// [`Holder::release`] (or drop).
struct Holder {
    release: Option<mpsc::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Holder {
    /// Returns once the lock is held.
    fn take(store: &BoardStore) -> Self {
        let path = store.path().to_path_buf();
        let (held, taken) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        let thread = thread::spawn(move || {
            let connection = Connection::open(path).unwrap();
            connection.execute_batch("BEGIN IMMEDIATE").unwrap();
            held.send(()).unwrap();
            // Held until told, or until the test gives up on it.
            let _told = released.recv_timeout(Duration::from_secs(30));
            connection.execute_batch("COMMIT").unwrap();
        });
        taken.recv_timeout(Duration::from_secs(30)).unwrap();
        Self {
            release: Some(release),
            thread: Some(thread),
        }
    }

    /// Lets go of the lock `after` from now, from another thread.
    fn release_after(&mut self, after: Duration) -> thread::JoinHandle<()> {
        let release = self.release.take().unwrap();
        thread::spawn(move || {
            thread::sleep(after);
            let _gone = release.send(());
        })
    }

    fn release(mut self) {
        drop(self.release.take());
        self.thread.take().unwrap().join().unwrap();
    }
}

impl Drop for Holder {
    fn drop(&mut self) {
        drop(self.release.take());
        if let Some(thread) = self.thread.take() {
            let _joined = thread.join();
        }
    }
}

/// The lock is held when the call begins and let go 150 ms later: the
/// call waits it out, busy, for no longer than the timeout.
#[test]
fn a_held_lock_is_measured_as_a_busy_wait() {
    let (_dir, store) = created();
    let mut holder = Holder::take(&store);
    let releaser = holder.release_after(Duration::from_millis(150));
    let (outcome, measure) = metered(|| store.transaction(false, |_| Ok(())));
    releaser.join().unwrap();
    holder.release();
    outcome.unwrap();
    assert!(measure.busy, "{measure:?}");
    assert_eq!(measure.transactions, 1, "{measure:?}");
    // The handler slept at least its first delay, inside `BEGIN`.
    assert!(measure.busy_wait >= Duration::from_millis(1), "{measure:?}");
    assert!(measure.lock_wait >= measure.busy_wait, "{measure:?}");
    assert!(measure.lock_wait < BUSY_TIMEOUT, "{measure:?}");
}

#[test]
fn a_free_lock_is_not_busy_and_the_waits_of_a_call_add_up() {
    let (_dir, store) = created();
    assert!(!active(), "no call is metered outside `metered`");
    let (inside, measure) = metered(|| {
        store.transaction(false, |_| Ok(())).unwrap();
        store.transaction(false, |_| Ok(())).unwrap();
        active()
    });
    assert!(inside, "a call is metered inside `metered`");
    assert!(!active(), "the scope ends with the call");
    assert!(!measure.busy, "{measure:?}");
    assert_eq!(measure.transactions, 2, "{measure:?}");
    assert_eq!(measure.busy_wait, Duration::ZERO, "{measure:?}");
    assert!(measure.lock_wait < Duration::from_secs(2), "{measure:?}");
    assert_eq!(measure.run_id, None, "the store alone reads no run");
}

/// A lock held for the whole call is refused with the text an unmetered
/// transaction gives, after the same wait: the busy handler slept the
/// whole timeout.
#[test]
fn a_metered_transaction_gives_up_as_sqlites_busy_timeout_does() {
    let (_dir, store) = created();
    let holder = Holder::take(&store);
    let started = Instant::now();
    let (outcome, measure) = metered(|| store.transaction(false, |_| Ok(())));
    let waited = started.elapsed();
    let plain = store.transaction(false, |_| Ok(()));
    holder.release();
    assert_eq!(outcome, plain, "the same refusal, metered or not");
    assert_eq!(
        outcome.unwrap_err().0,
        "coordination store unavailable or contended: database is locked"
    );
    assert!(measure.busy, "{measure:?}");
    assert!(measure.busy_wait >= BUSY_TIMEOUT, "{measure:?}");
    assert!(measure.lock_wait >= measure.busy_wait, "{measure:?}");
    assert!(waited >= BUSY_TIMEOUT, "{waited:?}");
    assert!(waited < Duration::from_secs(5), "{waited:?}");
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

/// Metering scopes nest: an inner call's measure is its own, and it is
/// added to the outer call's, which sat through it (#2303 review H2).
#[test]
fn a_nested_scope_is_its_own_and_adds_to_the_outer() {
    let (_dir, store) = created();
    let (inner, outer) = metered(|| {
        store.transaction(false, |_| Ok(())).unwrap();
        let (_, inner) = metered(|| store.transaction(false, |_| Ok(())).unwrap());
        assert!(active(), "the outer scope is restored");
        inner
    });
    assert!(!active());
    assert_eq!(inner.transactions, 1, "{inner:?}");
    assert_eq!(
        outer.transactions, 2,
        "the outer counts the inner's: {outer:?}"
    );
    assert!(outer.lock_wait >= inner.lock_wait, "{outer:?} {inner:?}");
    assert!(inner.lock_wait > Duration::ZERO, "{inner:?}");
}

/// The inner scope's busy wait reaches the outer one, and its busy flag.
#[test]
fn a_nested_busy_wait_is_the_outer_calls_too() {
    let (_dir, store) = created();
    let holder = Holder::take(&store);
    let (inner, outer) = metered(|| {
        let (_, inner) = metered(|| store.transaction(false, |_| Ok(())).unwrap_err());
        inner
    });
    holder.release();
    assert!(inner.busy && outer.busy, "{outer:?}");
    assert_eq!(outer.busy_wait, inner.busy_wait);
    assert_eq!(outer.lock_wait, inner.lock_wait);
}

/// The port: `work` runs exactly once; a call that began no transaction
/// has no measure (nothing was measured), one that did has its own.
#[test]
fn the_port_measures_only_a_call_that_began_a_transaction() {
    let (_dir, store) = created();
    let meter = SqliteBoardCallMeter;
    let mut runs = 0;
    assert_eq!(meter.metered(&mut || runs += 1), None);
    assert_eq!(runs, 1, "the work runs exactly once");
    let measure = meter
        .metered(&mut || store.transaction(false, |_| Ok(())).unwrap())
        .expect("a transaction was measured");
    assert_eq!(measure.transactions, 1, "{measure:?}");
    assert!(!active());
    assert_ne!(measure, CallMeasure::default());
}
