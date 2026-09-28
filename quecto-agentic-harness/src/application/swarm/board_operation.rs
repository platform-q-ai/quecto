//! The board's operation gate (#2270): red-phase skeleton.
use serde_json::Value;

use super::ports::{BoardEvents, BoardRepository, BoardRuns, BoardTransaction, Clock};
use crate::domain::swarm::{Access, BoardError, RunRecord};

pub(crate) fn atomic<T>(
    repository: &dyn BoardRepository,
    create: bool,
    work: impl FnOnce(&dyn BoardTransaction) -> Result<T, BoardError>,
) -> Result<T, BoardError> {
    let _ = (repository, create, work);
    Err(BoardError::new("not implemented yet (#2270)"))
}

pub(crate) fn operation<T>(
    repository: &dyn BoardRepository,
    clock: &dyn Clock,
    actor: &str,
    access: Access,
    work: impl FnOnce(&dyn BoardTransaction, &RunRecord) -> Result<T, BoardError>,
) -> Result<T, BoardError> {
    let _ = (repository, clock, actor, access, work);
    Err(BoardError::new("not implemented yet (#2270)"))
}

pub(crate) fn end(
    transaction: &(impl BoardRuns + BoardEvents + ?Sized),
    clock: &dyn Clock,
    actor: &str,
    outcome: &str,
    reason: &str,
) -> Result<(), BoardError> {
    let _ = (transaction, clock, actor, outcome, reason);
    Err(BoardError::new("not implemented yet (#2270)"))
}

pub(crate) fn detail<const N: usize>(entries: [(&str, Value); N]) -> Value {
    let _ = entries;
    Value::Null
}

pub(crate) fn text(value: &str) -> Value {
    let _ = value;
    Value::Null
}

#[cfg(test)]
#[path = "board_operation_tests.rs"]
mod tests;
