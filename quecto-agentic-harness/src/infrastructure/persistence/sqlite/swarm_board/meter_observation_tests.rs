//! Deterministic shared release-policy regressions.
use super::*;

const BEGIN: Scope = Scope {
    call: 1,
    connection: 2,
    transaction: 3,
    phase: Phase::Begin,
};
fn sample(scope: Scope, actual: u64, callback: i32) -> AccountedSample {
    AccountedSample {
        scope,
        callback,
        actual: Duration::from_millis(actual),
    }
}
#[test]
fn cancellation_without_held_ack_never_qualifies() {
    assert!(!release_qualified(false, BEGIN, sample(BEGIN, 100, 6)));
}
#[test]
fn error_notification_outside_begin_never_qualifies() {
    let scope = Scope {
        phase: Phase::Commit,
        ..BEGIN
    };
    assert!(!release_qualified(true, BEGIN, sample(scope, 100, 6)));
}
#[test]
fn invalid_callback_identity_never_qualifies() {
    assert!(!release_qualified(true, BEGIN, sample(BEGIN, 100, -1)));
}
#[test]
fn immutable_notification_does_not_gain_later_sleep_credit() {
    let earlier = sample(BEGIN, 53, 5);
    let later = sample(BEGIN, 103, 7);
    assert!(release_qualified(true, BEGIN, later));
    assert!(!release_qualified(true, BEGIN, earlier));
}
#[test]
fn actual_oversleep_is_credited_without_planned_budget_credit() {
    assert!(release_qualified(true, BEGIN, sample(BEGIN, 105, 5)));
}

#[test]
fn high_call_identity_bits_cannot_alias_the_intended_call() {
    let other = Scope {
        call: BEGIN.call + (1_u128 << 96),
        ..BEGIN
    };
    assert!(!release_qualified(true, BEGIN, sample(other, 100, 6)));
}
