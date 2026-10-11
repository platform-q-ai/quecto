//! The runs started from a task. A run that will never report its end (its
//! host died, or its holder walked away) is ended as `lost` by the task's
//! holder, which includes whoever takes an expired claim over; then the
//! task can be handed back or finished. S4's swarm fold-back uses this for
//! a run it finds dead.
use super::super::entities::claim::Identity;
use super::super::entities::task::Task;
use super::super::value_objects::rule_error::BoardRuleError;
use super::super::value_objects::timestamp::Timestamp;
use super::super::value_objects::vocabulary::RunOutcome;
use super::claims::{held_live_by, settle, stamp};

/// The task with its open run ended now as [`RunOutcome::Lost`], by the
/// holder of a live claim.
pub fn end_run_lost(
    task: &Task,
    actor: &Identity,
    now: &Timestamp,
) -> Result<Task, BoardRuleError> {
    held_live_by(task, actor, now)?;
    let at = stamp(task, now);
    let mut next = task.fields().clone();
    match next.runs.iter_mut().find(|run| run.is_open()) {
        Some(run) => {
            run.ended = Some(at.clone());
            run.outcome = Some(RunOutcome::Lost);
            settle(next, &at)
        }
        None => Err(BoardRuleError::NoOpenRun),
    }
}

#[cfg(test)]
#[path = "runs_tests.rs"]
mod tests;
