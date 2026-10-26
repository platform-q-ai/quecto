//! Claims last [`CLAIM_TTL_SECONDS`]: the holder renews, and anyone takes
//! over an expired claim on a task that may hold one.
//!
//! **Time.** Each rule is given the caller's clock, `now`. A change is
//! stamped with [`stamp`], `max(now, task.updated)`: the board orders its
//! changes by revision (#2705 B9), so a stamp is for display alone. It is
//! never pushed past the clock, and never runs backwards, so `updated`, a
//! new claim's `since` and `expires` and an ended run's `ended` stay
//! within what the schema holds. A claim runs from the caller's own clock
//! ([`claim_start`]), never from a stamp, so a task a fast clock stamped
//! ahead never stretches a claim a right clock takes on it. Whether a
//! claim has expired is judged by the caller's real `now`, so a client
//! whose clock runs fast sees claims expire early, and a claim such a
//! clock took lasts until its own expiry. S1c's write path bounds `now` against an outside
//! reading of the current time when one can be read: it refuses a `now`
//! more than [`CLOCK_AHEAD_SECONDS`] ahead of it.
use super::super::entities::claim::{CLAIM_TTL_SECONDS, Claim, Identity};
use super::super::entities::task::{Task, TaskFields};
use super::super::value_objects::rule_error::BoardRuleError;
use super::super::value_objects::schema_error::SchemaError;
use super::super::value_objects::telemetry::ClaimFacts;
use super::super::value_objects::timestamp::Timestamp;

/// The room past the latest time a write may build on that a plan, change
/// or review stamped by a clock near the bound may take.
pub const STAMP_SLACK_SECONDS: u64 = 30;

/// How far a clock may run ahead of an outside reading of the current
/// time before the board refuses to take its time.
pub const CLOCK_AHEAD_SECONDS: u64 = 60;

/// The holder's claim runs again from now for [`CLAIM_TTL_SECONDS`]: a
/// claim never spans more than that, so renewing restarts its window. An
/// expired claim is renewed too, until someone takes it over. A renewal
/// from a clock slower than the one that took the claim never shortens
/// it, nor moves its start back (#2705 B9 review round 2).
pub fn renew(task: &Task, holder: &Identity, now: &Timestamp) -> Result<Task, BoardRuleError> {
    let claim = held_or_lapsed_by(task, holder, now)?;
    let at = stamp(task, now);
    let fresh = new_claim(holder, task, now)?;
    let renewed = Claim {
        holder: claim.holder.clone(),
        since: fresh.since.max(claim.since.clone()),
        expires: fresh.expires.max(claim.expires.clone()),
    };
    assert!(
        renewed.since < renewed.expires && renewed.expires >= claim.expires,
        "a renewal never shortens the claim"
    );
    let mut next = task.fields().clone();
    next.claim = Some(renewed);
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
    let at = stamp(task, now);
    let mut next = fields.clone();
    next.claim = Some(new_claim(taker, task, now)?);
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

/// The task's claim, when `actor` holds it, live or expired, for the
/// holder-only actions an expired claim still allows its holder (renew,
/// release). Another's expired claim has a way on, as for every holder-only
/// action: take the task over (#2659, #2683).
pub(super) fn held_or_lapsed_by<'a>(
    task: &'a Task,
    actor: &Identity,
    now: &Timestamp,
) -> Result<&'a Claim, BoardRuleError> {
    match held_by(task, actor) {
        Err(BoardRuleError::NotHolder)
            if task
                .fields()
                .claim
                .as_ref()
                .is_some_and(|claim| !is_live(claim, now)) =>
        {
            Err(BoardRuleError::NotHolderExpired)
        }
        held => held,
    }
}

/// The task's claim, when `actor` holds it and it has not expired: the
/// holder renews an expired claim before working under it again.
pub(super) fn held_live_by<'a>(
    task: &'a Task,
    actor: &Identity,
    now: &Timestamp,
) -> Result<&'a Claim, BoardRuleError> {
    let claim = held_or_lapsed_by(task, actor, now)?;
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
/// who does. A claim's `since` is its taker's clock ([`claim_start`]),
/// which may read ahead of the writer's, so a `now` before it finds the
/// claim held for no time, never less (#2662).
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
/// [`STAMP_SLACK_SECONDS`]. The bound is inclusive, as the clock's own
/// is. A latest time so late that adding the slack overflows what the
/// board can spell tightens the bound to the latest time itself.
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

/// When a change to `task` happens, for display: the caller's `now`, or
/// the task's own `updated` if the caller's clock reads earlier. No stamp
/// is pushed past the clock, so two changes in one second share one: the
/// board's revision, never the stamp, orders them (#2705 B9).
pub(super) fn stamp(task: &Task, now: &Timestamp) -> Timestamp {
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

/// When a change to an artifact last changed at `updated` happens, for
/// display: `now`, or `updated` if `now` reads earlier. The one rule the
/// board's stamps follow (tasks, plans, changes, reviews and
/// discussions): a stamp never runs backwards and is never pushed past
/// the clock (#2705 B9: the board's revision orders changes).
pub(super) fn after_last(updated: &Timestamp, now: &Timestamp) -> Timestamp {
    let at = now.max(updated).clone();
    assert!(at >= *updated, "a stamp never runs backwards");
    at
}

/// When a claim on `task` taken or renewed at `now` begins (#2705 B9
/// review round 1): the caller's clock, never the task's stamp, so a task
/// a fast clock stamped ahead never stretches the claim; and never before
/// the task was created, as the schema holds a claim within its task's
/// life. It is never after the change's [`stamp`], which is no earlier
/// than `now`.
pub(super) fn claim_start(task: &Task, now: &Timestamp) -> Timestamp {
    let since = now.max(&task.fields().created).clone();
    assert!(
        since <= stamp(task, now),
        "a claim begins no later than the change that takes it"
    );
    since
}

/// A claim on `task` for `holder`, taken or renewed at `now`: from
/// [`claim_start`] until two hours from `now` by the caller's clock
/// (#2705 B9 review round 2), so a task created ahead never stretches it.
/// Only a task created two hours or more ahead holds a claim past that,
/// for the schema's least, a second past its start (a claim starts while
/// its task exists, and ends after it starts).
pub(super) fn new_claim(
    holder: &Identity,
    task: &Task,
    now: &Timestamp,
) -> Result<Claim, BoardRuleError> {
    let since = claim_start(task, now);
    let spelt = |at: Result<Timestamp, SchemaError>| {
        at.map_err(|error| BoardRuleError::Invalid(error.at("claim/expires")))
    };
    let two_hours = spelt(now.plus_seconds(CLAIM_TTL_SECONDS))?;
    let least = spelt(since.plus_seconds(1))?;
    let expires = two_hours.max(least);
    assert!(
        since < expires && expires <= spelt(since.plus_seconds(CLAIM_TTL_SECONDS))?,
        "a claim ends after it starts, within two hours of its start"
    );
    Ok(Claim {
        holder: holder.clone(),
        since,
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
