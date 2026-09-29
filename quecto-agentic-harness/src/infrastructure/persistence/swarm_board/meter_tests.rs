//! #2303: the lock a board transaction waits for is measured where it is
//! taken, only on the metered call's own repository, and waiting behaves
//! exactly as SQLite's own 500 ms busy timeout does. Nothing is ambient: a
//! call's measure is its own repository's, whatever else runs on the same
//! thread.
//!
//! The lock tests do not race a timer against the call (#2303 review L5):
//! the holder takes the lock before the call starts and keeps it until it
//! is told to let go, so the call is certain to find it held; the margins
//! are wide enough for a loaded 2-vCPU runner.
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use rusqlite::Connection;

use super::{SqliteBoardCallMeter, Tally, busy_delay, unwait_metered, wait_metered};
use crate::application::swarm::dto::{BoardLocation, CallMeasure};
use crate::application::swarm::ports::{BoardCallMeter, BoardRepository, MeteredCall};
use crate::domain::swarm::BoardError;
use crate::infrastructure::persistence::swarm_board::repository::SqliteBoardRepository;
use crate::infrastructure::persistence::swarm_board::store::{BUSY_TIMEOUT, BoardStore};

/// A created board, its plain (unmetered) repository and a meter over it.
fn created() -> (
    tempfile::TempDir,
    BoardStore,
    SqliteBoardRepository,
    SqliteBoardCallMeter,
) {
    let dir = tempfile::TempDir::new().unwrap();
    let location = BoardLocation {
        database: dir.path().join("swarm.sqlite"),
        checkout: dir.path().to_path_buf(),
    };
    let store = BoardStore::new(&location.database);
    store.transaction(true, |_| Ok(())).unwrap();
    let repository = SqliteBoardRepository::new(&location);
    let meter = SqliteBoardCallMeter::new(repository.clone());
    (dir, store, repository, meter)
}

/// One empty transaction on `call`'s own repository.
fn transact(call: &dyn MeteredCall) -> Result<(), BoardError> {
    call.atomic(false, &mut |_| Ok(()))
}

/// The call's measure, which a call that began a transaction has.
fn measured(call: &dyn MeteredCall) -> CallMeasure {
    call.measure().expect("a transaction began")
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
    let (_dir, store, _plain, meter) = created();
    let mut holder = Holder::take(&store);
    let releaser = holder.release_after(Duration::from_millis(150));
    let call = meter.open();
    let outcome = transact(&*call);
    releaser.join().unwrap();
    holder.release();
    outcome.unwrap();
    let measure = measured(&*call);
    assert!(measure.busy, "{measure:?}");
    assert_eq!(measure.transactions, 1, "{measure:?}");
    // The call sat out the ~150 ms hold inside `BEGIN` (#2303 round-3
    // review L2: the criterion's number, not merely a first delay).
    assert!(
        measure.busy_wait >= Duration::from_millis(100),
        "{measure:?}"
    );
    assert!(
        measure.lock_wait >= Duration::from_millis(100),
        "{measure:?}"
    );
    assert!(measure.lock_wait >= measure.busy_wait, "{measure:?}");
    assert!(measure.lock_wait < BUSY_TIMEOUT, "{measure:?}");
}

/// The busy handler is unregistered before its connection closes (#2303
/// round-3 review L6): once it is, a statement that finds the database
/// busy never reaches the tally, and nothing waits.
#[test]
fn an_unregistered_busy_handler_never_reaches_the_tally() {
    let (_dir, store, _plain, _meter) = created();
    let tally = Tally::new();
    let connection = Connection::open(store.path()).unwrap();
    wait_metered(&connection, &tally).unwrap();
    unwait_metered(&connection).unwrap();
    let holder = Holder::take(&store);
    let started = Instant::now();
    let refused = connection.execute_batch("BEGIN IMMEDIATE");
    let waited = started.elapsed();
    holder.release();
    assert!(refused.is_err(), "no handler retries: the lock is refused");
    assert!(
        !tally.held().busy,
        "the handler is gone: {:?}",
        tally.held()
    );
    assert!(
        waited < Duration::from_millis(250),
        "nothing waited: {waited:?}"
    );
    connection.close().unwrap();
}

/// A call's waits add up over its own transactions, and only its own: a
/// transaction on the plain repository, or on another call's, is not
/// counted, even on the same thread and in between.
#[test]
fn a_call_measures_only_its_own_transactions() {
    let (_dir, _store, plain, meter) = created();
    let (first, second) = (meter.open(), meter.open());
    assert_eq!(first.measure(), None, "nothing measured yet");
    transact(&*first).unwrap();
    plain.atomic(false, &mut |_| Ok(())).unwrap();
    transact(&*second).unwrap();
    transact(&*first).unwrap();
    let measure = measured(&*first);
    assert!(!measure.busy, "{measure:?}");
    assert_eq!(measure.transactions, 2, "{measure:?}");
    assert_eq!(measure.busy_wait, Duration::ZERO, "{measure:?}");
    assert!(measure.lock_wait < Duration::from_secs(2), "{measure:?}");
    assert_eq!(measure.run_id, None, "the store holds no run");
    assert_eq!(measured(&*second).transactions, 1);
    assert_eq!(meter.open().measure(), None, "a new call starts afresh");
}

/// Sets the board's run row to one whose id is `id`.
fn run_with_id(dir: &tempfile::TempDir, id: &str) {
    let connection = Connection::open(dir.path().join("swarm.sqlite")).unwrap();
    connection.execute("DELETE FROM run", []).unwrap();
    connection
        .execute(
            "INSERT INTO run(id,goal,constraints,criteria,coordinator,integrator,member_limit,deadline,status) \
             VALUES(?1,'','[]','[]','parent','parent',10,0,'setup')",
            [id],
        )
        .unwrap();
}

/// #2303 round-4 review L3: the run id a call records is the board's own
/// (`uuid4().hex`); a run id edited from outside, which could hold any
/// text, is recorded as none.
#[test]
fn only_a_generated_run_id_is_recorded() {
    let (dir, _store, _plain, meter) = created();
    let generated = "0123456789abcdef0123456789abcdef";
    run_with_id(&dir, generated);
    let call = meter.open();
    transact(&*call).unwrap();
    assert_eq!(measured(&*call).run_id.as_deref(), Some(generated));
    for edited in [
        "sk-livedeadbeef0001abcdefghijklmnop",
        "0123456789ABCDEF0123456789ABCDEF",
        "a secret the operator typed",
    ] {
        run_with_id(&dir, edited);
        let call = meter.open();
        transact(&*call).unwrap();
        transact(&*call).unwrap();
        let measure = measured(&*call);
        assert_eq!(measure.run_id, None, "{edited:?}: {measure:?}");
        assert_eq!(measure.transactions, 2);
    }
}

/// Two calls on two threads at once: each measures its own transactions.
#[test]
fn calls_on_two_threads_measure_apart() {
    let (_dir, _store, _plain, meter) = created();
    let counts: Vec<u32> = thread::scope(|scope| {
        let runs: Vec<_> = [3_u32, 5]
            .into_iter()
            .map(|runs| {
                let call = meter.open();
                scope.spawn(move || {
                    for _ in 0..runs {
                        transact(&*call).unwrap();
                    }
                    measured(&*call).transactions
                })
            })
            .collect();
        runs.into_iter().map(|run| run.join().unwrap()).collect()
    });
    assert_eq!(counts, [3, 5]);
}

/// A panic inside a metered transaction leaves no state behind: the call
/// still reads what it measured, and the next call starts afresh.
#[test]
fn a_panic_inside_a_call_leaves_nothing_behind() {
    let (_dir, _store, _plain, meter) = created();
    let call = meter.open();
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _never = call.atomic(false, &mut |_| panic!("the work panics"));
    }));
    assert!(panicked.is_err());
    assert_eq!(measured(&*call).transactions, 1, "the lock was taken");
    transact(&*call).unwrap();
    assert_eq!(measured(&*call).transactions, 2);
    let next = meter.open();
    transact(&*next).unwrap();
    assert_eq!(measured(&*next).transactions, 1);
}

/// A lock held for the whole call is refused with the text an unmetered
/// transaction gives, after the same wait: the busy handler slept the
/// whole timeout.
#[test]
fn a_metered_transaction_gives_up_as_sqlites_busy_timeout_does() {
    let (_dir, store, plain, meter) = created();
    let holder = Holder::take(&store);
    let started = Instant::now();
    let call = meter.open();
    let outcome = transact(&*call);
    let waited = started.elapsed();
    let unmetered = plain.atomic(false, &mut |_| Ok(()));
    holder.release();
    assert_eq!(outcome, unmetered, "the same refusal, metered or not");
    assert_eq!(
        outcome.unwrap_err().message(),
        "coordination store unavailable or contended: database is locked"
    );
    let measure = measured(&*call);
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
