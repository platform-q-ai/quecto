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

use super::{MeteredConnection, SqliteBoardCallMeter, Tally, busy_delay, unwait_metered};
use crate::application::swarm::dto::{BoardLocation, CallMeasure, RunRoles};
use crate::application::swarm::ports::{BoardCallMeter, BoardRepository, MeteredCall};
use crate::domain::swarm::value_objects::board_error::BoardError;
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

/// The lock is held until BEGIN has accounted at least 100 ms: the
/// call waits it out, busy, for no longer than the timeout.
#[test]
fn a_held_lock_is_measured_as_a_busy_wait() {
    let (_dir, store, _plain, meter) = created();
    let holder = super::observation::HeldLock::take(store.path().to_path_buf());
    let call = meter.open_observed(holder.observer());
    let outcome = transact(&*call);
    holder.finish();
    outcome.unwrap();
    let measure = measured(&*call);
    assert!(measure.busy, "{measure:?}");
    assert_eq!(measure.transactions, 1, "{measure:?}");
    // The call accounted at least 100 ms inside `BEGIN` (#2303 round-3
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
    let connection = MeteredConnection::new(connection, Some(&tally)).unwrap();
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

/// A metered call keeps the run's coordinator and integrator as the first
/// run row it read holds them (#2303 reconcile), whatever the run's id:
/// they name only the caller's role in the record, never a field of it.
/// A board with no run has none.
#[test]
fn the_run_roles_are_kept_for_the_callers_role() {
    let (dir, _store, _plain, meter) = created();
    let call = meter.open();
    transact(&*call).unwrap();
    assert_eq!(measured(&*call).run_roles, None, "the store holds no run");
    for id in [
        "0123456789abcdef0123456789abcdef",
        "an id edited from outside",
    ] {
        run_with_id(&dir, id);
        let call = meter.open();
        transact(&*call).unwrap();
        transact(&*call).unwrap();
        assert_eq!(
            measured(&*call).run_roles,
            Some(RunRoles {
                coordinator: Some("parent".to_owned()),
                integrator: Some("parent".to_owned()),
            }),
            "{id:?}"
        );
    }
}

/// #2313 (#2303 reconcile review nit): a call whose only run row read is
/// `run_status`'s, which names no integrator, still keeps the run's
/// roles, so a member-facing op reading only it records its caller's role.
#[test]
fn a_call_that_read_only_the_run_status_keeps_the_run_roles() {
    let (dir, _store, _plain, meter) = created();
    let generated = "0123456789abcdef0123456789abcdef";
    run_with_id(&dir, generated);
    let call = meter.open();
    call.atomic(false, &mut |board| board.run_status().map(|_| ()))
        .unwrap();
    let measure = measured(&*call);
    assert_eq!(measure.run_id.as_deref(), Some(generated));
    assert_eq!(
        measure.run_roles,
        Some(RunRoles {
            coordinator: Some("parent".to_owned()),
            integrator: Some("parent".to_owned()),
        })
    );
}

/// #2313 review nit: a call whose record's role is fixed (the harness's
/// own op, `_status` among them) does not read the run's roles: once its
/// run id is found, no telemetry statement of its own runs.
#[test]
fn a_call_with_a_fixed_role_reads_no_run_roles() {
    let (dir, _store, _plain, meter) = created();
    let generated = "0123456789abcdef0123456789abcdef";
    run_with_id(&dir, generated);
    let call = meter.open();
    call.role_fixed();
    call.atomic(false, &mut |board| board.run_status().map(|_| ()))
        .unwrap();
    let measure = measured(&*call);
    assert_eq!(measure.run_id.as_deref(), Some(generated));
    assert_eq!(measure.run_roles, None, "no roles were read for it");
    let unread = meter.open();
    unread.role_fixed();
    transact(&*unread).unwrap();
    assert_eq!(
        measured(&*unread).run_id.as_deref(),
        Some(generated),
        "a call that read no run row still reads its id"
    );
}

/// Deterministic accounting driver: gaps/setup have no account operation.
#[test]
fn accounted_notifications_are_immutable_attributed_and_post_unlock() {
    use super::observation::{Observation, Phase, Scope};
    use std::sync::{Arc, Mutex};
    let events = Arc::new(Mutex::new(Vec::new()));
    let kept = events.clone();
    let mut tally = Tally::new();
    tally.observe(Arc::new(move |event| kept.lock().unwrap().push(event)));
    let handler = super::Handler {
        tally: &tally,
        observation: Some(super::HandlerObservation {
            scope: Scope {
                call: 1,
                connection: 2,
                transaction: 3,
                phase: Phase::Setup,
            },
            phase: Arc::new(super::AtomicU8::new(1)),
            actual: Mutex::new((Phase::Setup, Duration::ZERO)),
        }),
    };
    for (callback, millis) in [1, 2, 5, 10, 15, 20].into_iter().enumerate() {
        handler.account(callback as i32, Duration::from_millis(millis));
    }
    handler.account(6, Duration::from_millis(52));
    let events = events.lock().unwrap();
    let Observation::Accounted(earlier) = events[5] else {
        panic!("accounted sample")
    };
    let Observation::Accounted(later) = events[6] else {
        panic!("accounted sample")
    };
    assert_eq!(earlier.actual, Duration::from_millis(53));
    assert_eq!(later.actual, Duration::from_millis(105));
    assert_eq!(
        later.scope,
        Scope {
            phase: Phase::Begin,
            ..handler.observation.as_ref().unwrap().scope
        }
    );
    assert_eq!(later.callback, 6);
    assert_eq!(tally.held().busy_wait, later.actual);
}

#[test]
fn observer_can_read_tally_after_accounting_without_deadlock() {
    // The observer holds a Weak to the call's own tally, avoiding a cycle.
    use super::observation::{Observation, Phase, Scope};
    use std::sync::{Arc, Mutex};
    let notifications = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = notifications.clone();
    let tally = Arc::new_cyclic(|weak: &std::sync::Weak<Tally>| {
        let weak = weak.clone();
        let mut tally = Tally::new();
        tally.observe(Arc::new(move |event| {
            if let Observation::Accounted(sample) = event {
                let tally = weak.upgrade().unwrap();
                let measured = tally
                    .measure
                    .try_lock()
                    .expect("notification after tally unlock");
                assert_eq!(measured.busy_wait, sample.actual);
                counted.fetch_add(1, super::Ordering::Relaxed);
            }
        }));
        tally
    });
    let handler = super::Handler {
        tally: &tally,
        observation: Some(super::HandlerObservation {
            scope: Scope {
                call: 1,
                connection: 2,
                transaction: 3,
                phase: Phase::Begin,
            },
            phase: Arc::new(super::AtomicU8::new(1)),
            actual: Mutex::new((Phase::Setup, Duration::ZERO)),
        }),
    };
    handler.account(0, Duration::from_millis(105));
    assert_eq!(notifications.load(super::Ordering::Relaxed), 1);
}

#[test]
fn observer_panic_is_contained_at_the_ffi_boundary_after_accounting() {
    use super::observation::{Phase, Scope};
    use std::sync::{Arc, Mutex};
    let mut tally = Tally::new();
    tally.observe(Arc::new(|_| panic!("observer panic")));
    let mut handler = super::Handler {
        tally: &tally,
        observation: Some(super::HandlerObservation {
            scope: Scope {
                call: 1,
                connection: 2,
                transaction: 3,
                phase: Phase::Begin,
            },
            phase: Arc::new(super::AtomicU8::new(1)),
            actual: Mutex::new((Phase::Setup, Duration::ZERO)),
        }),
    };
    // SAFETY: the same live boxed-argument contract, without invoking SQLite.
    let retry = unsafe { super::busy_callback(std::ptr::from_mut(&mut handler).cast(), 0) };
    assert_eq!(retry, 0, "panic means refusal, never unwind through C");
    assert!(tally.held().busy);
    assert!(tally.held().busy_wait >= Duration::from_millis(1));
}

#[test]
fn begin_observer_error_unwinds_raii_and_releases_connection() {
    use std::sync::Arc;
    let (_dir, _store, _plain, meter) = created();
    let observer = Arc::new(|_| panic!("begin observer panic"));
    let weak = Arc::downgrade(&observer);
    let call = meter.open_observed(observer);
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| transact(&*call)));
    assert!(outcome.is_err());
    assert_eq!(call.measure(), None, "BEGIN was never attempted");
    drop(call);
    assert!(weak.upgrade().is_none(), "observer owned solely by call");
    transact(&*meter.open()).unwrap();
}

#[test]
fn holder_cancellation_is_joined_and_does_not_fake_success() {
    let (_dir, store, _plain, meter) = created();
    let holder = super::observation::HeldLock::take(store.path().to_path_buf());
    let observer = holder.observer();
    drop(holder);
    drop(observer);
    transact(&*meter.open()).unwrap();
    let holder = super::observation::HeldLock::take(store.path().to_path_buf());
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| holder.finish()));
    assert!(
        outcome.is_err(),
        "cancellation is not qualified successful release"
    );
    transact(&*meter.open()).unwrap();
}

#[test]
fn setup_and_commit_accounting_cannot_credit_the_begin_sample() {
    use super::observation::{Observation, Phase, Scope};
    use std::sync::{Arc, Mutex};
    let events = Arc::new(Mutex::new(Vec::new()));
    let kept = events.clone();
    let mut tally = Tally::new();
    tally.observe(Arc::new(move |event| kept.lock().unwrap().push(event)));
    let handler = super::Handler {
        tally: &tally,
        observation: Some(super::HandlerObservation {
            scope: Scope {
                call: 1,
                connection: 2,
                transaction: 3,
                phase: Phase::Setup,
            },
            phase: Arc::new(super::AtomicU8::new(0)),
            actual: Mutex::new((Phase::Setup, Duration::ZERO)),
        }),
    };
    handler.account(0, Duration::from_millis(100));
    handler
        .observation
        .as_ref()
        .unwrap()
        .phase
        .store(1, super::Ordering::Release);
    handler.account(0, Duration::from_millis(53));
    handler
        .observation
        .as_ref()
        .unwrap()
        .phase
        .store(3, super::Ordering::Release);
    handler.account(0, Duration::from_millis(7));
    let events = events.lock().unwrap();
    let Observation::Accounted(begin) = events[1] else {
        panic!("BEGIN sample")
    };
    let Observation::Accounted(commit) = events[2] else {
        panic!("COMMIT sample")
    };
    assert_eq!(begin.scope.phase, Phase::Begin);
    assert_eq!(
        begin.actual,
        Duration::from_millis(53),
        "setup credit is excluded"
    );
    assert_eq!(
        commit.actual,
        Duration::from_millis(7),
        "BEGIN credit is excluded"
    );
    assert_eq!(
        tally.held().busy_wait,
        Duration::from_millis(160),
        "existing call-wide math is unchanged"
    );
    assert_eq!(tally.held().lock_wait, Duration::ZERO);
}

#[test]
fn call_connection_and_transaction_identities_are_never_reused() {
    let mut first = Tally::new();
    let mut second = Tally::new();
    first.observe(std::sync::Arc::new(|_| {}));
    second.observe(std::sync::Arc::new(|_| {}));
    assert_ne!(
        first.observation.as_ref().unwrap().call_id,
        second.observation.as_ref().unwrap().call_id
    );
    let one = MeteredConnection::new(Connection::open_in_memory().unwrap(), Some(&first)).unwrap();
    let two = MeteredConnection::new(Connection::open_in_memory().unwrap(), Some(&first)).unwrap();
    let one = one
        .handler
        .as_ref()
        .unwrap()
        .observation
        .as_ref()
        .unwrap()
        .scope;
    let two = two
        .handler
        .as_ref()
        .unwrap()
        .observation
        .as_ref()
        .unwrap()
        .scope;
    assert_eq!(one.call, two.call);
    assert_ne!(one.connection, two.connection);
    assert_ne!(one.transaction, two.transaction);
}

#[test]
fn holder_startup_error_is_joined_without_a_lock_acknowledgement() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("absent-parent").join("swarm.sqlite");
    let outcome = std::panic::catch_unwind(|| super::observation::HeldLock::take(path));
    assert!(outcome.is_err());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn ffi_null_argument_and_exhausted_planned_budget_refuse_without_sleep_credit() {
    use super::observation::{Phase, Scope};
    use std::sync::{Arc, Mutex};
    // SAFETY: null is explicitly refused without dereferencing it.
    assert_eq!(unsafe { super::busy_callback(std::ptr::null_mut(), 0) }, 0);
    let tally = Tally::new();
    let handler = super::Handler {
        tally: &tally,
        observation: Some(super::HandlerObservation {
            scope: Scope {
                call: 1,
                connection: 2,
                transaction: 3,
                phase: Phase::Begin,
            },
            phase: Arc::new(super::AtomicU8::new(1)),
            actual: Mutex::new((Phase::Setup, Duration::ZERO)),
        }),
    };
    assert!(!handler.busy(14));
    assert_eq!(tally.held().busy_wait, Duration::ZERO);
    handler.account(0, Duration::MAX);
    handler.account(1, Duration::from_millis(1));
    assert_eq!(
        tally.held().busy_wait,
        Duration::MAX,
        "saturating totals preserved"
    );
}

#[test]
fn retained_call_identity_is_a_full_per_call_uuid_not_an_ambient_counter() {
    let mut first = Tally::new();
    first.observe(std::sync::Arc::new(|_| {}));
    let retained = first.observation.as_ref().unwrap().call_id;
    drop(first);
    let mut next = Tally::new();
    next.observe(std::sync::Arc::new(|_| {}));
    assert_ne!(
        retained,
        next.observation.as_ref().unwrap().call_id,
        "retained call identity cannot alias a later call"
    );
    assert_eq!(
        uuid::Uuid::from_u128(retained).get_version_num(),
        4,
        "call identity is locally generated UUIDv4, not ambient sequence state"
    );
}
