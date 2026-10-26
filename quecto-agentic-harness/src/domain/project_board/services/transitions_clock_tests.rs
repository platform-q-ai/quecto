//! A claim runs by the caller's clock, never a stamp (#2705 B9 review
//! rounds 1 to 4).
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

/// `task(status)` created and last written at `created`, its claim (when
/// it holds one) running from then for a second.
fn held_from(status: TaskStatus, created: &str) -> Task {
    let mut fields = task(status).into_fields();
    (fields.created, fields.updated) = (at(created), at(created));
    if let Some(claim) = fields.claim.as_mut() {
        claim.since = at(created);
        claim.expires = at(created).plus_seconds(1).unwrap();
    }
    Task::new(fields).unwrap()
}

/// #2705 B9 review rounds 3 and 4 (F3): a task created more than
/// [`CLOCK_AHEAD_SECONDS`] ahead of the caller's clock is never claimed,
/// taken over or renewed (a start renews or claims): its claim could only
/// start at its creation and so outlast the caller's two hours. The
/// refusal names the clock skew, `task_clock_ahead`, and when it clears.
/// The bound is exact: a task created 60 s ahead is claimed, 61 s ahead
/// is refused.
#[test]
fn a_task_created_past_the_clock_bound_is_refused_a_claim_as_task_clock_ahead() {
    use super::super::claims::{CLOCK_AHEAD_SECONDS, renew, take_over};
    let skewed = |created: &str, until: &str| BoardRuleError::TaskClockAhead {
        created: at(created),
        until: at(until),
    };
    let ada = person("Ada");
    let nine = at("09:00:00");
    let claim =
        |created: &str| move_task(&created_at(created), Claimed, &ada, (&[], &[]), &[], &nine);
    assert_eq!(claim("23:00:00"), Err(skewed("23:00:00", "22:59:00")));
    let held = held_from(InProgress, "23:00:00");
    assert_eq!(
        renew(&held, &ada, &nine),
        Err(skewed("23:00:00", "22:59:00"))
    );
    assert_eq!(
        take_over(&held, &person("Bob"), &nine),
        Err(skewed("23:00:00", "22:59:00"))
    );
    assert_eq!(nine.plus_seconds(CLOCK_AHEAD_SECONDS), Ok(at("09:01:00")));
    assert!(claim("09:01:00").is_ok(), "60 s ahead is within the bound");
    assert_eq!(
        claim("09:01:01"),
        Err(skewed("09:01:01", "09:00:01")),
        "61 s ahead is past it"
    );
    assert_eq!(
        skewed("23:00:00", "22:59:00").to_string(),
        "the task was created at 2026-10-10T23:00:00Z, ahead of this clock: a clock is wrong, \
         or wait until 2026-10-10T22:59:00Z"
    );
}

/// #2705 B9 review round 4: the clock bound is judged after a take-over's
/// and a renewal's own refusals (the task's status, who holds it), so a
/// task created ahead is told those first.
#[test]
fn the_clock_bound_is_judged_after_the_status_and_holder_refusals() {
    use super::super::claims::{renew, take_over};
    let (ada, bob, nine) = (person("Ada"), person("Bob"), at("09:00:00"));
    for status in [Draft, Ready, Done, Archived] {
        let ahead = held_from(status, "23:00:00");
        assert_eq!(
            take_over(&ahead, &bob, &nine),
            Err(BoardRuleError::NotTakeable(status)),
            "{status:?}"
        );
    }
    let held = held_from(InProgress, "23:00:00");
    assert_eq!(
        take_over(&held, &ada, &nine),
        Err(BoardRuleError::AlreadyHolder)
    );
    assert_eq!(renew(&held, &bob, &nine), Err(BoardRuleError::NotHolder));
    let unclaimed = held_from(Review, "23:00:00");
    let mut fields = unclaimed.into_fields();
    fields.claim = None;
    let unclaimed = Task::new(fields).unwrap();
    assert_eq!(
        take_over(&unclaimed, &bob, &nine),
        Err(BoardRuleError::NoClaim)
    );
    assert_eq!(renew(&unclaimed, &ada, &nine), Err(BoardRuleError::NoClaim));
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
