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

/// A claim is live until the moment it expires; from then on it is
/// expired and can be taken over.
pub fn is_live(claim: &Claim, now: &Timestamp) -> bool {
    *now < claim.expires
}

/// What a change by `writer` at `now` finds of `task`'s claim (epic
/// #2481 T4), `None` when it holds none: how long it has been held and
/// has left, and whether the writer holds it ([`Identity::is`]); never
/// who does.
pub fn claim_facts(task: &Task, writer: &Identity, now: &Timestamp) -> Option<ClaimFacts> {
    task.fields().claim.as_ref().map(|claim| ClaimFacts {
        held_s: now.seconds_since(&claim.since),
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
