//! The pull requests a task links. Completing a task when they merge is a
//! status move ([`complete_merged`](super::transitions::complete_merged));
//! unlinking one closed without merging is the holder's.
use super::super::entities::claim::Identity;
use super::super::entities::task::Task;
use super::super::value_objects::rule_error::BoardRuleError;
use super::super::value_objects::timestamp::Timestamp;
use super::claims::{clock_keeps_up, held_live_by, settle};

/// The task without pull request `pr`, unlinked by the holder of a live
/// claim (a pull request closed without merging no longer delivers it).
pub fn remove_pr(
    task: &Task,
    holder: &Identity,
    pr: u64,
    now: &Timestamp,
) -> Result<Task, BoardRuleError> {
    clock_keeps_up(task, now)?;
    held_live_by(task, holder, now)?;
    let mut next = task.fields().clone();
    match next.prs.iter().position(|linked| *linked == pr) {
        Some(index) => {
            next.prs.remove(index);
            settle(next, now)
        }
        None => Err(BoardRuleError::PrNotLinked(pr)),
    }
}

#[cfg(test)]
#[path = "pull_requests_tests.rs"]
mod tests;
