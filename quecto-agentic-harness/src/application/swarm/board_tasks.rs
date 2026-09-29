//! How the task use cases (#2272) read a task: `Tasks._task`, with the
//! status it derives, and `Tasks._owned`, the claim check every owner
//! operation makes. Capability-internal helpers, not a use case and not a
//! port.
use serde_json::Value;

use super::dto::TaskRow;
use super::ports::BoardTasks;
use crate::domain::swarm::{BoardError, python_equal};

/// The statuses of a held claim (`swarm_repository.ACTIVE_CLAIM`).
pub(crate) const HELD_CLAIM: [&str; 3] = ["claimed", "blocked", "submitted"];

/// `Tasks._task(db, task_id)`: the row as `dict(row)` with its JSON columns
/// loaded; a `ready` task whose dependencies are not all `completed` reads
/// `blocked` with the blocker `unmet dependencies` (in place, so the row
/// keeps its column order). The dependencies are read one at a time and
/// the first incomplete one decides, as Python's `any` does.
///
/// # Errors
/// `unknown task`, or the store's refusal (a task id it cannot bind).
pub(crate) fn read_task(
    transaction: &(impl BoardTasks + ?Sized),
    task_id: &Value,
) -> Result<TaskRow, BoardError> {
    let Some(mut task) = transaction.task(task_id)? else {
        return Err(BoardError::new("unknown task"));
    };
    if task.text("status") == Some("ready") && unmet(transaction, task.dependencies())? {
        task.set("status", Value::from("blocked"));
        task.set("blocker", Value::from("unmet dependencies"));
    }
    Ok(task)
}

/// Whether a dependency is not `completed`, reading them in order until
/// the first that is not (a missing row is not).
fn unmet(
    transaction: &(impl BoardTasks + ?Sized),
    dependencies: &[Value],
) -> Result<bool, BoardError> {
    for dependency in dependencies {
        let completed = transaction.task_status(dependency)?.as_deref() == Some("completed");
        if completed {
            continue;
        }
        return Ok(true);
    }
    Ok(false)
}

/// `Tasks._owned(db, task_id, token)` for `member`: the task, when its
/// token and owner equal the given ones by Python's `==` and it holds a
/// claim.
///
/// # Errors
/// `stale or unowned claim`, or [`read_task`]'s refusal.
pub(crate) fn owned(
    transaction: &(impl BoardTasks + ?Sized),
    task_id: &Value,
    token: &Value,
    member: &str,
) -> Result<TaskRow, BoardError> {
    let task = read_task(transaction, task_id)?;
    let equals = |column: &str, given: &Value| {
        task.get(column)
            .is_some_and(|stored| python_equal(stored, given))
    };
    let held = task
        .text("status")
        .is_some_and(|status| HELD_CLAIM.contains(&status));
    if equals("token", token) && equals("owner", &Value::from(member)) && held {
        Ok(task)
    } else {
        Err(BoardError::new("stale or unowned claim"))
    }
}

#[cfg(test)]
#[path = "board_tasks_tests.rs"]
mod tests;
