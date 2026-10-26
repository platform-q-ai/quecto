//! A claim runs by the caller's clock, never a stamp (#2705 B9 review
//! rounds 1 to 3).
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

/// #2705 B9 review rounds 2 and 3 (F3): a task a fast clock created
/// 30 s ahead and claimed at 09:00 is claimed from 09:00:30 (as the schema
/// holds a claim within its task's life) only until 11:00, two hours by
/// the caller's clock: at 11:00 anyone takes it over.
#[test]
fn a_task_created_within_the_clock_bound_is_claimed_for_the_callers_two_hours() {
    let ready = created_at("09:00:30");
    let near = move_task(
        &ready,
        Claimed,
        &person("Ada"),
        (&[], &[]),
        &[],
        &at("09:00:00"),
    )
    .unwrap();
    let claim = near.fields().claim.clone().unwrap();
    assert_eq!(
        (&claim.since, &claim.expires),
        (&at("09:00:30"), &at("11:00:00"))
    );
    let taken = super::super::claims::take_over(&near, &person("Bob"), &at("11:00:00"));
    assert!(taken.is_ok(), "{taken:?}");
}

/// #2705 B9 review round 3 (F3): a task created more than
/// [`CLOCK_AHEAD_SECONDS`] ahead of the caller's clock is never claimed,
/// taken over or renewed (a start renews or claims): its claim could only
/// start at its creation and so outlast the caller's two hours. The
/// refusal names the clock skew, `task_clock_ahead`, and a task created
/// at the bound is claimed.
#[test]
fn a_task_created_past_the_clock_bound_is_refused_a_claim_as_task_clock_ahead() {
    use super::super::claims::{CLOCK_AHEAD_SECONDS, renew, take_over};
    let skewed = BoardRuleError::TaskClockAhead {
        created: at("23:00:00"),
    };
    let ada = person("Ada");
    let nine = at("09:00:00");
    let ready = created_at("23:00:00");
    let claimed = move_task(&ready, Claimed, &ada, (&[], &[]), &[], &nine);
    assert_eq!(claimed, Err(skewed.clone()));
    let mut held = task(InProgress).into_fields();
    held.created = at("23:00:00");
    held.updated = at("23:00:00");
    if let Some(claim) = held.claim.as_mut() {
        (claim.since, claim.expires) = (at("23:00:00"), at("23:00:01"));
    }
    let held = Task::new(held).unwrap();
    assert_eq!(renew(&held, &ada, &nine), Err(skewed.clone()));
    assert_eq!(take_over(&held, &person("Bob"), &nine), Err(skewed));
    assert_eq!(nine.plus_seconds(CLOCK_AHEAD_SECONDS), Ok(at("09:01:00")));
    let at_bound = created_at("09:01:00");
    assert!(move_task(&at_bound, Claimed, &ada, (&[], &[]), &[], &nine).is_ok());
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
