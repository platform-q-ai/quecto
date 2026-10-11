//! The pull requests a task links (red stub).
use super::super::entities::claim::Identity;
use super::super::entities::task::Task;
use super::super::value_objects::rule_error::BoardRuleError;
use super::super::value_objects::timestamp::Timestamp;

pub fn remove_pr(
    _task: &Task,
    _holder: &Identity,
    _pr: u64,
    _now: &Timestamp,
) -> Result<Task, BoardRuleError> {
    Err(BoardRuleError::NoClaim)
}

#[cfg(test)]
#[path = "pull_requests_tests.rs"]
mod tests;
