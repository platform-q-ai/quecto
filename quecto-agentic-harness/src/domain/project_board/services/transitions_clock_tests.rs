//! A claim runs by the caller's clock, never a stamp (#2705 B9 review
//! rounds 1 and 2).
use super::*;
use crate::domain::project_board::services::fixtures_tests::{at, last_written, person, task};

/// #2705 B9 review round 1: a task a fast clock stamped at 23:00 and
/// claimed at 09:00 by a right one is claimed from 09:00 to 11:00, by the
/// caller's clock: its stamp (for display) never stretches the claim, so
/// at 11:00 anyone takes it over. A renewal and a take-over alike.
#[test]
fn a_task_stamped_ahead_never_stretches_a_claim_taken_on_it() {
    let late = at("23:00:00");
    let ready = last_written(task(Ready), &late);
    let claimed = move_task(
        &ready,
        Claimed,
        &person("Ada"),
        (&[], &[]),
        &[],
        &at("09:00:00"),
    )
    .unwrap();
    let claim = claimed.fields().claim.clone().unwrap();
    assert_eq!(
        (&claim.since, &claim.expires, &claimed.fields().updated),
        (&at("09:00:00"), &at("11:00:00"), &late),
        "claimed by the caller's clock, stamped no earlier than the task"
    );
    let taken = super::super::claims::take_over(&claimed, &person("Bob"), &at("11:00:00"))
        .expect("the claim lapsed at 11:00");
    let claim = taken.fields().claim.clone().unwrap();
    assert_eq!(
        (&claim.since, &claim.expires),
        (&at("11:00:00"), &at("13:00:00"))
    );
    let renewed = super::super::claims::renew(&taken, &person("Bob"), &at("12:00:00")).unwrap();
    let claim = renewed.fields().claim.clone().unwrap();
    assert_eq!(
        (&claim.since, &claim.expires),
        (&at("12:00:00"), &at("14:00:00"))
    );
}

/// `task(Ready)` as a fast clock created it, at `created`.
fn created_at(created: &str) -> Task {
    let mut fields = task(Ready).into_fields();
    (fields.created, fields.updated) = (at(created), at(created));
    Task::new(fields).unwrap()
}

/// #2705 B9 review round 2 (F3): a task a fast clock created at 09:30 and
/// claimed at 09:00 is claimed from 09:30 (as the schema holds a claim
/// within its task's life) yet only until 11:00, two hours by the
/// caller's clock: at 11:00 anyone takes it over. A task created far
/// ahead holds its claim no longer than the schema's least, a second.
#[test]
fn a_task_created_ahead_never_stretches_a_claim_past_the_callers_two_hours() {
    let claimed = |created: &str| {
        let ready = created_at(created);
        let ada = person("Ada");
        let claimed = move_task(&ready, Claimed, &ada, (&[], &[]), &[], &at("09:00:00")).unwrap();
        claimed
            .fields()
            .claim
            .clone()
            .map(|claim| (claimed, claim))
            .unwrap()
    };
    let (near, claim) = claimed("09:30:00");
    assert_eq!(
        (&claim.since, &claim.expires),
        (&at("09:30:00"), &at("11:00:00"))
    );
    let taken = super::super::claims::take_over(&near, &person("Bob"), &at("11:00:00"));
    assert!(taken.is_ok(), "{taken:?}");
    let (_, claim) = claimed("23:00:00");
    assert_eq!(
        (&claim.since, &claim.expires),
        (&at("23:00:00"), &at("23:00:01"))
    );
}

/// #2705 B9 review round 2: a renewal from a clock slower than the one
/// that took the claim never shortens it, nor moves its start back.
#[test]
fn a_renewal_from_a_slower_clock_never_shortens_a_live_claim() {
    let held = task(InProgress);
    let renewed = super::super::claims::renew(&held, &person("Ada"), &at("08:30:00")).unwrap();
    let claim = renewed.fields().claim.clone().unwrap();
    assert_eq!(
        (&claim.since, &claim.expires),
        (&at("09:00:00"), &at("11:00:00")),
        "the claim was 09:00 to 11:00"
    );
    let later = super::super::claims::renew(&held, &person("Ada"), &at("10:00:00")).unwrap();
    let claim = later.fields().claim.clone().unwrap();
    assert_eq!(
        (&claim.since, &claim.expires),
        (&at("10:00:00"), &at("12:00:00"))
    );
}
