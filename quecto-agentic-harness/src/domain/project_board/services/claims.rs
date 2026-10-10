//! Claims last [`CLAIM_TTL_SECONDS`]: the holder renews, and anyone takes
//! over an expired claim on a claimed or in-progress task.
use super::super::entities::claim::{CLAIM_TTL_SECONDS, Claim, Identity};
use super::super::entities::task::{Task, TaskFields};
use super::super::value_objects::rule_error::BoardRuleError;
use super::super::value_objects::timestamp::Timestamp;
use super::super::value_objects::vocabulary::TaskStatus;

/// The holder's claim runs again from now for [`CLAIM_TTL_SECONDS`]: a
/// claim never spans more than that, so renewing restarts its window.
pub fn renew(task: &Task, holder: &Identity, now: &Timestamp) -> Result<Task, BoardRuleError> {
    let claim = held_by(task, holder)?;
    let mut next = task.fields().clone();
    next.claim = Some(Claim {
        holder: claim.holder.clone(),
        ..new_claim(holder, now)?
    });
    settle(next, now)
}

/// Someone else takes an expired claim on a claimed or in-progress task.
pub fn take_over(task: &Task, taker: &Identity, now: &Timestamp) -> Result<Task, BoardRuleError> {
    let fields = task.fields();
    let takeable = matches!(fields.status, TaskStatus::Claimed | TaskStatus::InProgress);
    let claim = match &fields.claim {
        Some(claim) if takeable => claim,
        _ => return Err(BoardRuleError::NotTakeable(fields.status)),
    };
    if claim.holder.is(taker) {
        return Err(BoardRuleError::AlreadyHolder);
    }
    if claim.expires > *now {
        return Err(BoardRuleError::ClaimLive);
    }
    let mut next = fields.clone();
    next.claim = Some(new_claim(taker, now)?);
    settle(next, now)
}

/// The task's claim, when `actor` holds it (by email, ignoring case).
pub(super) fn held_by<'a>(task: &'a Task, actor: &Identity) -> Result<&'a Claim, BoardRuleError> {
    match &task.fields().claim {
        Some(claim) if claim.holder.is(actor) => Ok(claim),
        Some(_) => Err(BoardRuleError::NotHolder),
        None => Err(BoardRuleError::NoClaim),
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
    next.updated = now.clone();
    Task::new(next).map_err(BoardRuleError::Invalid)
}

#[cfg(test)]
#[path = "claims_tests.rs"]
mod tests;
