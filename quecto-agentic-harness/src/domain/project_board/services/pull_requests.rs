//! The pull requests a task links. Completing a task when they merge is a
//! status move ([`complete_merged`](super::transitions::complete_merged));
//! linking one, or unlinking one deliberately (a wrong or abandoned pull
//! request), is the holder's: a task with linked pull requests closes on
//! merge, and once the holder unlinks them all, Review→Done is theirs.
use super::super::entities::claim::Identity;
use super::super::entities::task::Task;
use super::super::value_objects::rule_error::BoardRuleError;
use super::super::value_objects::timestamp::Timestamp;
use super::changes::Linked;
use super::claims::{held_live_by, settle, stamp};

/// The task linking pull request `pr` too, linked by the holder of a live
/// claim. `board` is everything linked on the board: a pull request is
/// linked to one task once, whichever way, so one a linked change holds,
/// or another task links the old way, is refused.
pub fn add_pr(
    task: &Task,
    holder: &Identity,
    pr: u64,
    board: Linked<'_>,
    now: &Timestamp,
) -> Result<Task, BoardRuleError> {
    may_link(task, holder, now)?;
    let by_change = board
        .changes
        .iter()
        .find(|change| change.is_linked() && change.fields().pr == Some(pr))
        .map(|change| (change.fields().task.clone(), Some(change.fields().number)));
    let by_task = board
        .tasks
        .iter()
        .find(|other| other.fields().id != task.fields().id && other.fields().prs.contains(&pr))
        .map(|other| (other.fields().id.clone(), None));
    match by_change.or(by_task) {
        None => {}
        Some((task, change)) => return Err(BoardRuleError::ChangeAlreadyLinked { task, change }),
    }
    let mut next = task.fields().clone();
    match next.prs.contains(&pr) {
        true => Err(BoardRuleError::PrAlreadyLinked(pr)),
        false => {
            next.prs.push(pr);
            settle(next, &stamp(task, now)?)
        }
    }
}

/// Whether `holder` may link a pull request to `task` at `now`, judged on
/// the task alone (#2686): only the holder of a live claim links one. What
/// the rest of the board links is [`add_pr`]'s to judge, so a caller that
/// must read the whole board for it asks this first, and a link the task's
/// own rule refuses is refused as such, whatever else the board holds.
pub fn may_link(task: &Task, holder: &Identity, now: &Timestamp) -> Result<(), BoardRuleError> {
    held_live_by(task, holder, now).map(|_| ())
}

/// The task without pull request `pr`, unlinked deliberately by the
/// holder of a live claim (a wrong or abandoned pull request no longer
/// delivers it).
pub fn remove_pr(
    task: &Task,
    holder: &Identity,
    pr: u64,
    now: &Timestamp,
) -> Result<Task, BoardRuleError> {
    held_live_by(task, holder, now)?;
    let mut next = task.fields().clone();
    match next.prs.iter().position(|linked| *linked == pr) {
        Some(index) => {
            next.prs.remove(index);
            settle(next, &stamp(task, now)?)
        }
        None => Err(BoardRuleError::PrNotLinked(pr)),
    }
}

#[cfg(test)]
#[path = "pull_requests_tests.rs"]
mod tests;
