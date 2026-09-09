use super::*;
use crate::domain::inference_admission::{AttemptObservation, GroupActivity};

fn g(name: &str) -> GroupId {
    GroupId::new(name).unwrap()
}

fn attempt(group: &str, phase: AdmissionPhase, elapsed_ms: u64) -> AttemptObservation {
    AttemptObservation {
        alias: "acct".into(),
        group: g(group),
        phase,
        elapsed_ms,
    }
}

fn activity(attempts: Vec<AttemptObservation>) -> AdmissionActivity {
    let waiting = attempts
        .iter()
        .filter(|a| matches!(a.phase, AdmissionPhase::Waiting { .. }))
        .count();
    let admitted = attempts.len() - waiting;
    let mut activity = AdmissionActivity {
        waiting,
        admitted,
        observed_at_ms: 1_000,
        ..AdmissionActivity::default()
    };
    for (id, a) in attempts.into_iter().enumerate() {
        activity.groups.entry(a.group.clone()).or_default();
        activity.attempts.insert(id as u64, a);
    }
    activity
}

#[test]
fn no_waiting_attempt_yields_no_verdict() {
    let idle = activity(vec![]);
    assert_eq!(waiting_verdict(&idle), None);
    let admitted_only = activity(vec![attempt(
        "g",
        AdmissionPhase::Admitted { since_ms: 0 },
        50,
    )]);
    assert_eq!(waiting_verdict(&admitted_only), None);
}

#[test]
fn the_longest_waiting_attempt_names_the_group_and_cause() {
    let view = activity(vec![
        attempt("fast", AdmissionPhase::Waiting { since_ms: 900 }, 100),
        attempt("slow", AdmissionPhase::Waiting { since_ms: 200 }, 800),
        attempt("fast", AdmissionPhase::Admitted { since_ms: 0 }, 1_000),
    ]);
    let verdict = waiting_verdict(&view).unwrap();
    assert_eq!(verdict.waiting, 2);
    assert_eq!(verdict.longest_wait_ms, 800);
    assert_eq!(verdict.attribution, Some((g("slow"), WaitCause::Occupancy)));
}

#[test]
fn cooldown_states_become_causes_with_remaining_time() {
    let mut view = activity(vec![attempt(
        "g",
        AdmissionPhase::Waiting { since_ms: 0 },
        1_000,
    )]);
    let cases = [
        (
            Some(CooldownState::Until { until_ms: 4_500 }),
            WaitCause::Cooldown {
                remaining_ms: 3_500,
            },
        ),
        (
            Some(CooldownState::Until { until_ms: 900 }),
            WaitCause::Cooldown { remaining_ms: 0 },
        ),
        (
            Some(CooldownState::Unknown { since_ms: 10 }),
            WaitCause::ThrottledIndefinitely,
        ),
        (Some(CooldownState::Unavailable), WaitCause::Unavailable),
        (None, WaitCause::Occupancy),
    ];
    for (cooldown, expected) in cases {
        view.groups.insert(
            g("g"),
            GroupActivity {
                cooldown,
                last_refusal: None,
            },
        );
        assert_eq!(
            waiting_verdict(&view).unwrap().attribution,
            Some((g("g"), expected)),
            "{cooldown:?}"
        );
    }
}

#[test]
fn a_fully_hidden_queue_reports_an_unknown_wait_without_inventing_one() {
    let mut view = activity(vec![]);
    view.waiting = 3;
    view.hidden = 3;
    view.groups.insert(g("g"), GroupActivity::default());
    let verdict = waiting_verdict(&view).unwrap();
    assert_eq!(verdict.waiting, 3);
    assert_eq!(verdict.longest_wait_ms, 0);
    assert_eq!(
        verdict.attribution, None,
        "never attributed to a guessed group"
    );
}
