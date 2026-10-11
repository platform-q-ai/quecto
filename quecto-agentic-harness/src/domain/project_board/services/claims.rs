//! Claims last [`CLAIM_TTL_SECONDS`]: the holder renews, and anyone takes
//! over an expired claim on a task that may hold one.
use super::super::entities::claim::{CLAIM_TTL_SECONDS, Claim, Identity};
use super::super::entities::task::{Task, TaskFields};
use super::super::value_objects::rule_error::BoardRuleError;
use super::super::value_objects::timestamp::Timestamp;

/// The holder's claim runs again from now for [`CLAIM_TTL_SECONDS`]: a
/// claim never spans more than that, so renewing restarts its window. An
/// expired claim is renewed too, until someone takes it over.
pub fn renew(task: &Task, holder: &Identity, now: &Timestamp) -> Result<Task, BoardRuleError> {
    clock_keeps_up(task, now)?;
    let claim = held_by(task, holder)?;
    let mut next = task.fields().clone();
    next.claim = Some(Claim {
        holder: claim.holder.clone(),
        ..new_claim(holder, now)?
    });
    settle(next, now)
}

/// Someone else takes an expired claim on a task that may hold one
/// (claimed, in progress, in review or blocked).
pub fn take_over(task: &Task, taker: &Identity, now: &Timestamp) -> Result<Task, BoardRuleError> {
    clock_keeps_up(task, now)?;
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
    let mut next = fields.clone();
    next.claim = Some(new_claim(taker, now)?);
    settle(next, now)
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
    let claim = held_by(task, actor)?;
    if is_live(claim, now) {
        Ok(claim)
    } else {
        Err(BoardRuleError::ClaimExpired)
    }
}

/// A claim is live until the moment it expires; from then on it is
/// expired and can be taken over.
pub(super) fn is_live(claim: &Claim, now: &Timestamp) -> bool {
    *now < claim.expires
}

/// Every change happens at or after the task's last one, so `updated`
/// never runs backwards.
pub(super) fn clock_keeps_up(task: &Task, now: &Timestamp) -> Result<(), BoardRuleError> {
    let updated = &task.fields().updated;
    if updated <= now {
        Ok(())
    } else {
        Err(BoardRuleError::ClockBehind {
            now: now.clone(),
            updated: updated.clone(),
        })
    }
}

pub(super) fn new_claim(holder: &Identity, now: &Timestamp) -> Result<Claim, BoardRuleError> {
    let expires = now
        .plus_seconds(CLAIM_TTL_SECONDS)
        .map_err(BoardRuleError::Invalid)?;
    Ok(Claim {
        holder: holder.clone(),
        since: now.clone(),
        expires,
    })
}

/// Stamps the change and builds the task again, so no rule leaves one
/// that breaks the schema.
pub(super) fn settle(mut next: TaskFields, now: &Timestamp) -> Result<Task, BoardRuleError> {
    debug_assert!(
        next.updated <= *now,
        "every rule checks the clock before it changes a task"
    );
    next.updated = now.clone();
    Task::new(next).map_err(BoardRuleError::Invalid)
}

#[cfg(test)]
#[path = "claims_tests.rs"]
mod tests;
