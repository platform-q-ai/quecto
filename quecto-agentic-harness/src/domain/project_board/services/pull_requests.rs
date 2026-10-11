//! The pull requests a task links. Completing a task when they merge is a
//! status move ([`complete_merged`](super::transitions::complete_merged));
//! linking one, or unlinking one deliberately (a wrong or abandoned pull
//! request), is the holder's: a task with linked pull requests closes on
//! merge, and once the holder unlinks them all, Review→Done is theirs.
use super::super::entities::change::Change;
use super::super::entities::claim::Identity;
use super::super::entities::task::Task;
use super::super::value_objects::rule_error::BoardRuleError;
use super::super::value_objects::timestamp::Timestamp;
use super::claims::{held_live_by, settle, stamp};

/// The task linking pull request `pr` too, linked by the holder of a live
/// claim. `changes` are the task's (as the board holds them): a pull
/// request one of them links is not linked the old way too.
pub fn add_pr(
    task: &Task,
    holder: &Identity,
    pr: u64,
    changes: &[Change],
    now: &Timestamp,
) -> Result<Task, BoardRuleError> {
    held_live_by(task, holder, now)?;
    let own = changes
        .iter()
        .all(|change| change.fields().task == task.fields().id);
    let linking = changes
        .iter()
        .find(|change| change.is_linked() && change.fields().pr == Some(pr));
    match (own, linking) {
        (true, None) => {}
        (false, _) => return Err(BoardRuleError::NotThisTasks),
        (true, Some(change)) => {
            return Err(BoardRuleError::ChangeAlreadyLinked {
                task: task.fields().id.clone(),
                change: Some(change.fields().number),
            });
        }
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
