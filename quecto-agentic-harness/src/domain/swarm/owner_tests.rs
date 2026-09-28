//! Ported from `tests/swarm_policy_test.py::OwnerStateBehavior`.
use super::*;

#[test]
fn owner_state_is_affirmative_over_every_member_status() {
    let now = 1000.0;
    let (recent, stale) = (now - OWNER_IDLE_AFTER + 1.0, now - OWNER_IDLE_AFTER);
    let state = |status, lost, activity| owner_state(status, lost, activity, now, OWNER_IDLE_AFTER);
    assert_eq!(state(Some("live"), false, Some(recent)), OwnerState::Active);
    assert_eq!(state(Some("live"), false, Some(stale)), OwnerState::Idle);
    assert_eq!(state(Some("live"), false, None), OwnerState::Unknown);
    assert_eq!(state(Some("live"), true, Some(recent)), OwnerState::Lost);
    assert_eq!(
        state(Some("reserved"), false, Some(recent)),
        OwnerState::Reserved
    );
    assert_eq!(
        state(Some("reserved"), true, Some(recent)),
        OwnerState::Lost
    );
    assert_eq!(state(Some("dead"), false, Some(recent)), OwnerState::Dead);
    assert_eq!(state(Some("dead"), true, None), OwnerState::Dead);
    for status in [
        None,
        Some(""),
        Some("lost"),
        Some("suspended"),
        Some("LIVE"),
        Some("future-status"),
    ] {
        assert_eq!(
            state(status, false, Some(recent)),
            OwnerState::Unknown,
            "{status:?}"
        );
        assert_eq!(
            state(status, true, Some(recent)),
            OwnerState::Unknown,
            "{status:?}"
        );
    }
    assert_eq!(
        owner_state(Some("live"), false, Some(0.0), 20.0, 30.0),
        OwnerState::Active
    );
    assert!(!OWNER_STATES.contains(&"suspended"));
    assert_eq!(ADDRESSABLE_OWNER_STATES, ["active", "idle"]);
}

#[test]
fn owner_state_names_match_the_python_table() {
    let states = [
        OwnerState::Active,
        OwnerState::Idle,
        OwnerState::Reserved,
        OwnerState::Lost,
        OwnerState::Dead,
        OwnerState::Unknown,
    ];
    assert_eq!(states.map(OwnerState::as_str), OWNER_STATES);
    assert_eq!(
        OWNER_STATES,
        ["active", "idle", "reserved", "lost", "dead", "unknown"]
    );
    let addressable: Vec<&str> = states
        .into_iter()
        .filter(|state| state.addressable())
        .map(OwnerState::as_str)
        .collect();
    assert_eq!(addressable, ADDRESSABLE_OWNER_STATES);
}

#[test]
fn a_fractional_last_activity_compares_exactly() {
    // 1000.0 - 700.1 is 299.9 minus a rounding error: still active, as in Python.
    let active = owner_state(Some("live"), false, Some(700.1), 1000.0, OWNER_IDLE_AFTER);
    assert!(1000.0 - 700.1 < OWNER_IDLE_AFTER);
    assert_eq!(active, OwnerState::Active);
    let idle = owner_state(Some("live"), false, Some(699.9), 1000.0, OWNER_IDLE_AFTER);
    assert_eq!(idle, OwnerState::Idle);
    // A NaN activity is never at least `idle_after` old, as in Python.
    let nan = owner_state(
        Some("live"),
        false,
        Some(f64::NAN),
        1000.0,
        OWNER_IDLE_AFTER,
    );
    assert_eq!(nan, OwnerState::Active);
}

#[test]
fn owner_recovery_names_the_coordinator_move_per_state() {
    assert_eq!(
        owner_recovery(OwnerState::Dead),
        "recover(task) or revoke(task, reason)"
    );
    assert!(owner_recovery(OwnerState::Lost).contains("resume the run"));
    assert_eq!(
        owner_recovery(OwnerState::Lost),
        "resume the run (agent_cmd swarm_control resume), then revoke(task, reason)"
    );
    for state in [
        OwnerState::Reserved,
        OwnerState::Unknown,
        OwnerState::Active,
        OwnerState::Idle,
    ] {
        assert_eq!(owner_recovery(state), "revoke(task, reason)", "{state:?}");
    }
}

#[test]
fn idle_transition_is_the_future_instant_or_none() {
    assert_eq!(
        idle_transition(Some(100.0), 150.0, OWNER_IDLE_AFTER),
        Some(100.0 + OWNER_IDLE_AFTER)
    );
    assert_eq!(
        idle_transition(Some(100.0), 100.0 + OWNER_IDLE_AFTER, OWNER_IDLE_AFTER),
        None
    );
    assert_eq!(idle_transition(None, 150.0, OWNER_IDLE_AFTER), None);
    assert_eq!(idle_transition(Some(100.0), 105.0, 10.0), Some(110.0));
    // Strictly in the future: an instant one ulp ahead still counts.
    let at: f64 = 100.0 + OWNER_IDLE_AFTER;
    assert_eq!(
        idle_transition(Some(100.0), at - at * f64::EPSILON, OWNER_IDLE_AFTER),
        Some(at)
    );
    assert_eq!(
        idle_transition(Some(100.0), at + 1.0, OWNER_IDLE_AFTER),
        None
    );
}
