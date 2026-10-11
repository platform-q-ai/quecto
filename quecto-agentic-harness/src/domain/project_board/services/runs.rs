//! The runs started from a task (red stub).
use super::super::entities::claim::Identity;
use super::super::entities::task::Task;
use super::super::value_objects::rule_error::BoardRuleError;
use super::super::value_objects::timestamp::Timestamp;

pub fn end_run_lost(
    _task: &Task,
    _actor: &Identity,
    _now: &Timestamp,
) -> Result<Task, BoardRuleError> {
    Err(BoardRuleError::NoClaim)
}

#[cfg(test)]
#[path = "runs_tests.rs"]
mod tests;
