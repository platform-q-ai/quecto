//! Claims last [`CLAIM_TTL_SECONDS`]: the holder renews, and anyone takes
//! over an expired claim on a task that may hold one.
//!
//! **Time.** Each rule is given the caller's clock, `now`. A change is
//! stamped with [`stamp`], `max(now, task.updated + 1 s)`, a hybrid
//! logical clock: `updated`, a new claim's `since` and `expires` and an
//! ended run's `ended` always move forward, and a write from a clock that ran
//! ahead never locks the task for clients whose clocks are right. Whether a
//! claim has expired is judged by the caller's real `now`, so a client
//! whose clock runs fast sees claims expire early, and a claim stamped by
//! such a clock lasts until its stamped expiry. S1c's write path bounds
//! both against an outside reading of the current time when one can be
//! read: it refuses a `now`, or a task `updated`, more than
//! [`CLOCK_AHEAD_SECONDS`] ahead of it.
use super::super::entities::claim::{CLAIM_TTL_SECONDS, Claim, Identity};
use super::super::entities::task::{Task, TaskFields};
use super::super::value_objects::rule_error::BoardRuleError;
use super::super::value_objects::telemetry::ClaimFacts;
use super::super::value_objects::timestamp::Timestamp;

/// The room the strictly-forward stamp needs past the latest time a
/// write may build on: each change stamps a second past the task it read,
/// so a run of quick changes from a clock near the bound edges past it,
/// and the next honest writer must still build on the task.
pub const STAMP_SLACK_SECONDS: u64 = 30;

/// How far a clock may run ahead of an outside reading of the current
/// time before the board refuses to take its time, or to build on a task
/// it stamped.
pub const CLOCK_AHEAD_SECONDS: u64 = 60;

/// The holder's claim runs again from now for [`CLAIM_TTL_SECONDS`]: a
/// claim never spans more than that, so renewing restarts its window. An
/// expired claim is renewed too, until someone takes it over.
pub fn renew(task: &Task, holder: &Identity, now: &Timestamp) -> Result<Task, BoardRuleError> {
    let claim = held_by(task, holder)?;
    let at = stamp(task, now)?;
    let mut next = task.fields().clone();
    next.claim = Some(Claim {
        holder: claim.holder.clone(),
        ..new_claim(holder, &at)?
    });
    settle(next, &at)
}

/// Someone else takes an expired claim on a task that may hold one
/// (claimed, in progress, in review or blocked).
pub fn take_over(task: &Task, taker: &Identity, now: &Timestamp) -> Result<Task, BoardRuleError> {
    let fields = task.fields();
    let claim = match (&fields.claim, fields.status.may_hold_claim()) {
        (Some(claim), true) => claim,
        (None, true) => return Err(BoardRuleError::NoClaim),
        (_, false) => return Err(BoardRuleError::NotTakeable(fields.status)),
    };
    if claim.holder.is(taker) {
        return Err(BoardRuleError::AlreadyHolder);
    }
    if is_live(claim, now) {
        return Err(BoardRuleError::ClaimLive);
    }
    let at = stamp(task, now)?;
    let mut next = fields.clone();
    next.claim = Some(new_claim(taker, &at)?);
    settle(next, &at)
}

/// The task's claim, when `actor` holds it (by email, ignoring case),
/// live or expired.
pub(super) fn held_by<'a>(task: &'a Task, actor: &Identity) -> Result<&'a Claim, BoardRuleError> {
    match &task.fields().claim {
        Some(claim) if claim.holder.is(actor) => Ok(claim),
        Some(_) => Err(BoardRuleError::NotHolder),
        None => Err(BoardRuleError::NoClaim),
    }
}

/// The task's claim, when `actor` holds it and it has not expired: the
/// holder renews an expired claim before working under it again.
pub(super) fn held_live_by<'a>(
    task: &'a Task,
    actor: &Identity,
    now: &Timestamp,
) -> Result<&'a Claim, BoardRuleError> {
    // Another's expired claim has a way on: take the task over (#2659).
    let claim = match held_by(task, actor) {
        Err(BoardRuleError::NotHolder)
            if task
                .fields()
                .claim
                .as_ref()
                .is_some_and(|claim| !is_live(claim, now)) =>
        {
            return Err(BoardRuleError::NotHolderExpired);
        }
        held => held?,
    };
    if is_live(claim, now) {
        Ok(claim)
    } else {
        Err(BoardRuleError::ClaimExpired)
    }
}

/// Whether `holder`'s renewal of `task`'s claim at `now` is due (#2656):
/// it is, unless `holder` holds the claim and `now` is before halfway
/// through its own lease ([`renew_after`]). Anything else is the
/// renewal's own rule to judge.
pub fn renewal_due(task: &Task, holder: &Identity, now: &Timestamp) -> bool {
    match &task.fields().claim {
        Some(claim) if claim.holder.is(holder) => *now >= halfway(claim),
        Some(_) | None => true,
    }
}

/// When the claim `task` holds is next renewed (#2656): halfway through
/// its own lease; `None` for a task that holds none. A task holds only a
/// claim its board validated, whose lease is never negative.
pub fn renew_after(task: &Task) -> Option<Timestamp> {
    task.fields().claim.as_ref().map(halfway)
}

/// Halfway through `claim`'s lease, `since + (expires - since) / 2`,
/// which lies between the two, so the board spells it whenever it spells
/// the claim: for a task's validated claim only.
fn halfway(claim: &Claim) -> Timestamp {
    let lease = claim.expires.seconds_since(&claim.since);
    let half = u64::try_from(lease / 2).unwrap_or(0);
    let at = claim
        .since
        .plus_seconds(half)
        .expect("halfway lies before the claim's expiry, which the board spells");
    assert!(
        claim.since <= at && at <= claim.expires,
        "halfway is within the lease"
    );
    at
}

/// A claim is live until the moment it expires; from then on it is
/// expired and can be taken over.
pub fn is_live(claim: &Claim, now: &Timestamp) -> bool {
    *now < claim.expires
}

/// What a change by `writer` at `now` finds of `task`'s claim (epic
/// #2481 T4), `None` when it holds none: how long it has been held and
/// has left, and whether the writer holds it ([`Identity::is`]); never
/// who does. A claim's `since` is a [`stamp`], which may run a second or
/// more past the wall clock, so a `now` before it finds the claim held
/// for no time, never less (#2662).
pub fn claim_facts(task: &Task, writer: &Identity, now: &Timestamp) -> Option<ClaimFacts> {
    task.fields().claim.as_ref().map(|claim| ClaimFacts {
        held_s: now.seconds_since(&claim.since).max(0),
        left_s: claim.expires.seconds_since(now),
        holder_is_writer: claim.holder.is(writer),
    })
}

/// Whether something stamped `updated` may be built on, given the
/// `latest` time an outside reference allows (`None`: no reference, and
/// nothing is refused): no later than that time plus
/// [`STAMP_SLACK_SECONDS`], the room the strictly-forward stamp needs.
/// The bound is inclusive, as the clock's own is. A latest time so late
/// that adding the slack overflows what the board can spell tightens the
/// bound to the latest time itself.
pub fn within_slack(updated: &Timestamp, latest: Option<&Timestamp>) -> bool {
    match latest {
        None => true,
        Some(latest) => {
            let bound = latest
                .plus_seconds(STAMP_SLACK_SECONDS)
                .unwrap_or_else(|_| latest.clone());
            *updated <= bound
        }
    }
}

/// When a change to `task` happens: the caller's `now`, or a second past
/// the task's own `updated` if the caller's clock reads no later. Every
/// change moves `updated` strictly forward, so two changes in one second,
/// or one from a clock running behind, never share a stamp (a caller's
/// `expected_updated` tells them apart).
///
/// # Errors
/// A task stamped at the last moment the board can spell.
pub(super) fn stamp(task: &Task, now: &Timestamp) -> Result<Timestamp, BoardRuleError> {
    after_last(&task.fields().updated, now)
}

/// The time a change to another artifact of `task` (a plan, say) builds
/// on: the caller's `now`, or, if the caller's clock reads earlier, the
/// latest of the task's own `updated` and `stamps` (every artifact of the
/// task of the kind changed, as the board holds them), so no artifact of
/// one task is stamped before the task's last change or a sibling's, and
/// stamps never run backwards on one task (#2662).
pub(super) fn not_before<'a>(
    task: &Task,
    stamps: impl IntoIterator<Item = &'a Timestamp>,
    now: &Timestamp,
) -> Timestamp {
    let floor = stamps
        .into_iter()
        .fold(&task.fields().updated, |latest, stamp| latest.max(stamp));
    let at = now.max(floor).clone();
    assert!(
        at >= task.fields().updated,
        "a task's stamps never run backwards"
    );
    at
}

/// When a change to an artifact last changed at `updated` happens: `now`,
/// or a second past `updated` if `now` reads no later. The one rule the
/// board's stamps follow (tasks, plans, changes, reviews and
/// discussions), so each
/// change moves its `updated` strictly forward.
///
/// # Errors
/// An artifact stamped at the last moment the board can spell.
pub(super) fn after_last(
    updated: &Timestamp,
    now: &Timestamp,
) -> Result<Timestamp, BoardRuleError> {
    let next = updated
        .plus_seconds(1)
        .map_err(|error| BoardRuleError::Invalid(error.at("updated")))?;
    let at = now.max(&next).clone();
    assert!(at > *updated, "a change moves updated strictly forward");
    Ok(at)
}

/// A claim for `holder` from `at` (a [`stamp`]) for two hours.
pub(super) fn new_claim(holder: &Identity, at: &Timestamp) -> Result<Claim, BoardRuleError> {
    let expires = at
        .plus_seconds(CLAIM_TTL_SECONDS)
        .map_err(|error| BoardRuleError::Invalid(error.at("claim/expires")))?;
    Ok(Claim {
        holder: holder.clone(),
        since: at.clone(),
        expires,
    })
}

/// Stamps the change at `at` (a [`stamp`]) and builds the task again, so
/// no rule leaves one that breaks the schema.
pub(super) fn settle(mut next: TaskFields, at: &Timestamp) -> Result<Task, BoardRuleError> {
    debug_assert!(next.updated <= *at, "a stamp never runs backwards");
    next.updated = at.clone();
    Task::new(next).map_err(BoardRuleError::Invalid)
}

#[cfg(test)]
#[path = "claims_tests.rs"]
mod tests;
