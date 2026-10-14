//! `MeteredCall` on the SQLite adapter (#2303): a call is the repository
//! it measures. It has measured nothing until a transaction begins through
//! it, and a lock held for ~150 ms is measured as a lock wait and a busy
//! wait of at least 100 ms each (the criterion's number, round-3 review
//! L2).
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use quecto::application::swarm::ports::{BoardCallMeter, BoardRepository};

use super::board_call_meter::board;

#[path = "../support/held_lock_timer_policy.rs"]
mod held_lock_timer_policy;

#[test]
fn a_call_measures_the_transactions_begun_through_it() {
    let (_dir, _plain, meter) = board();
    let call = meter.open();
    assert_eq!(call.measure(), None, "nothing began");
    let repository: std::sync::Arc<dyn BoardRepository> = call.clone();
    repository.atomic(false, &mut |_| Ok(())).unwrap();
    assert_eq!(call.measure().unwrap().transactions, 1);
}

#[test]
fn a_held_lock_is_measured_as_its_wait() {
    let (dir, _plain, meter) = board();
    let (held, taken) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let path = dir.path().join("swarm.sqlite");
    let holder = thread::spawn(move || {
        let connection = rusqlite::Connection::open(path).unwrap();
        connection.execute_batch("BEGIN IMMEDIATE").unwrap();
        held.send(()).unwrap();
        let _told = released.recv_timeout(Duration::from_secs(30));
        connection.execute_batch("COMMIT").unwrap();
    });
    taken.recv_timeout(Duration::from_secs(30)).unwrap();
    let releaser = thread::spawn(move || {
        held_lock_timer_policy::timer_release(Duration::from_millis(150), thread::sleep, || {
            release.send(()).unwrap();
        });
    });
    let call = meter.open();
    call.atomic(false, &mut |_| Ok(())).unwrap();
    releaser.join().unwrap();
    holder.join().unwrap();
    let measure = call.measure().expect("a transaction began");
    assert!(measure.busy, "{measure:?}");
    assert!(
        measure.lock_wait >= Duration::from_millis(100),
        "{measure:?}"
    );
    assert!(
        measure.busy_wait >= Duration::from_millis(100),
        "{measure:?}"
    );
}

/// Policy-only fake evidence: this does not exercise the production meter,
/// observer lifetime, tally mutex, SQLite callback or FFI unwind boundary.
mod held_lock_release_policy {
    use std::cell::Cell;

    use super::{Duration, held_lock_timer_policy};

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Phase {
        Begin,
        Work,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct Scope {
        call: u64,
        connection: u64,
        transaction: u64,
        phase: Phase,
    }

    const INTENDED: Scope = Scope {
        call: 1,
        connection: 2,
        transaction: 3,
        phase: Phase::Begin,
    };

    #[derive(Clone, Copy, Debug)]
    struct AccountedSample {
        scope: Scope,
        planned_ms: u64,
        actual_accounted_ms: u64,
    }

    #[derive(Clone, Copy, Debug)]
    struct ReleaseSnapshot {
        held_ack: bool,
        intended: Scope,
        begin_entered: bool,
        sample: Option<AccountedSample>,
        outside_sleep_gap_ms: u64,
    }

    fn sample(scope: Scope, planned_ms: u64, actual_accounted_ms: u64) -> AccountedSample {
        AccountedSample {
            scope,
            planned_ms,
            actual_accounted_ms,
        }
    }

    fn assert_release_qualified(
        accounted: Option<AccountedSample>,
        begin_entered: bool,
        held_ack: bool,
        outside_sleep_gap_ms: u64,
    ) {
        // The gate is explicit immutable driver state. Advancing the independent
        // timer does not advance the contender into BEGIN or create a sample.
        let timer_fired = Cell::new(false);
        let releases = Cell::new(0_u32);
        let observed = Cell::new(None);
        held_lock_timer_policy::timer_release(
            Duration::from_millis(150),
            |after| {
                assert_eq!(after, Duration::from_millis(150));
                timer_fired.set(true);
            },
            || {
                assert!(timer_fired.get(), "the actual shared timer policy fired");
                releases.set(releases.get() + 1);
                observed.set(Some(ReleaseSnapshot {
                    held_ack,
                    intended: INTENDED,
                    begin_entered,
                    sample: accounted,
                    outside_sleep_gap_ms,
                }));
            },
        );
        assert_eq!(
            releases.get(),
            1,
            "observe the shared policy's real release event"
        );
        let released = observed
            .get()
            .expect("timer policy emitted a release snapshot");
        // An affirmative oracle, NOT an implementation of a new release policy.
        // Only ACTUALLY ALREADY accounted sleep can qualify; neither planned
        // duration nor outside-sleep time is credited. Scope equality includes
        // call, connection, transaction and BEGIN phase.
        let qualifies = match released.sample {
            Some(measured) => {
                released.held_ack
                    && released.begin_entered
                    && measured.scope == released.intended
                    && measured.actual_accounted_ms >= 100
            }
            None => false,
        };
        assert!(
            qualifies,
            "successful release requires held acknowledgement and matching already-accounted BEGIN sleep >=100ms; observed {released:?}"
        );
        // Fields deliberately not used as qualification inputs, but retained
        // immutably in the event evidence for diagnosis.
        let _uncredited = (
            released.sample.map(|s| s.planned_ms),
            released.outside_sleep_gap_ms,
        );
    }

    #[test]
    fn timer_cannot_release_contender_gated_before_begin() {
        assert_release_qualified(None, false, true, 0);
    }

    #[test]
    fn held_acknowledgement_is_required() {
        assert_release_qualified(Some(sample(INTENDED, 100, 100)), true, false, 0);
    }

    macro_rules! wrong_scope {
        ($name:ident, $field:ident, $value:expr) => {
            #[test]
            fn $name() {
                let scope = Scope {
                    $field: $value,
                    ..INTENDED
                };
                assert_release_qualified(Some(sample(scope, 100, 100)), true, true, 0);
            }
        };
    }

    wrong_scope!(wrong_call_cannot_qualify, call, 9);
    wrong_scope!(wrong_connection_cannot_qualify, connection, 9);
    wrong_scope!(wrong_transaction_cannot_qualify, transaction, 9);
    wrong_scope!(wrong_phase_cannot_qualify, phase, Phase::Work);

    #[test]
    fn first_callback_without_accounted_sleep_cannot_qualify() {
        assert_release_qualified(Some(sample(INTENDED, 1, 0)), true, true, 0);
    }

    #[test]
    fn six_sleeps_53ms_and_outside_gap_7ms_cannot_qualify() {
        let six_sleeps = [1_u64, 2, 5, 10, 15, 20];
        let actual = six_sleeps.into_iter().sum();
        assert_eq!(actual, 53);
        assert_release_qualified(Some(sample(INTENDED, 53, actual)), true, true, 7);
    }

    #[test]
    fn planned_100ms_with_only_53ms_accounted_cannot_qualify() {
        assert_release_qualified(Some(sample(INTENDED, 100, 53)), true, true, 0);
    }

    /// Compatibility controls validate the oracle, not production accounting.
    #[test]
    fn matching_actual_accounted_100ms_is_compatible() {
        assert_release_qualified(Some(sample(INTENDED, 100, 100)), true, true, 0);
    }

    #[test]
    fn actual_oversleep_105ms_despite_plan_53ms_is_compatible() {
        assert_release_qualified(Some(sample(INTENDED, 53, 105)), true, true, 0);
    }
}
