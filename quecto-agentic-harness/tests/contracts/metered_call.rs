//! `MeteredCall` on the SQLite adapter (#2303): a call is the repository
//! it measures. It has measured nothing until a transaction begins through
//! it, and a lock held through accounted BEGIN sleeps is measured as a lock wait and a busy
//! wait of at least 100 ms each (the criterion's number, round-3 review
//! L2).
use std::time::Duration;

use quecto::application::swarm::ports::{
    board_repository::BoardRepository, board_telemetry::BoardCallMeter,
};

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
    let holder = held_lock_timer_policy::HeldLock::take(dir.path().join("swarm.sqlite"));
    let call = meter.open_observed(holder.observer());
    let outcome = call.atomic(false, &mut |_| Ok(()));
    holder.finish();
    outcome.unwrap();
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

/// Deterministic evidence for the same policy as both fixtures. This does not exercise the production meter,
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
        let releases = Cell::new(0_u32);
        let observed = Cell::new(None);
        let scope = |scope: Scope| held_lock_timer_policy::Scope {
            call: u128::from(scope.call),
            connection: scope.connection,
            transaction: scope.transaction,
            phase: match scope.phase {
                Phase::Begin => held_lock_timer_policy::Phase::Begin,
                Phase::Work => held_lock_timer_policy::Phase::Work,
            },
        };
        let qualifies_policy = accounted.is_some_and(|sample| {
            begin_entered
                && held_lock_timer_policy::release_qualified(
                    held_ack,
                    scope(INTENDED),
                    held_lock_timer_policy::AccountedSample {
                        scope: scope(sample.scope),
                        callback: 6,
                        actual: Duration::from_millis(sample.actual_accounted_ms),
                    },
                )
        });
        if qualifies_policy {
            releases.set(releases.get() + 1);
            observed.set(Some(ReleaseSnapshot {
                held_ack,
                intended: INTENDED,
                begin_entered,
                sample: accounted,
                outside_sleep_gap_ms,
            }));
        }
        let expected = held_ack
            && begin_entered
            && accounted.is_some_and(|sample| {
                sample.scope == INTENDED && sample.actual_accounted_ms >= 100
            });
        assert_eq!(
            releases.get(),
            u32::from(expected),
            "unqualified holder must remain held; qualified holder must release exactly once"
        );
        let Some(released) = observed.get() else {
            return;
        };
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
        let mut lifecycle = held_lock_timer_policy::ReleaseLifecycle::new(true);
        assert!(
            !lifecycle.qualified(),
            "shared lifecycle must keep holder locked while contender is gated before BEGIN"
        );
        let begin = held_lock_timer_policy::Scope {
            call: 1,
            connection: 2,
            transaction: 3,
            phase: held_lock_timer_policy::Phase::Begin,
        };
        assert!(
            !lifecycle.observe(held_lock_timer_policy::Observation::Begin(begin)),
            "BEGIN entry without accounted receipt cannot release holder"
        );
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
