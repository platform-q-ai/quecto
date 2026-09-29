//! The blocker, submission and verification methods of the board dispatch
//! (#2272): their Python signatures and their serving. Every argument
//! reaches the use case as the JSON value passed (Python binds a task id
//! or a token untyped and type-checks the rest at run time), and each
//! method answers `None`. The decision names whether the call changed the
//! task or found it already as asked; the record names the task by the id
//! its row holds (#2303), as the task methods' records do.
use serde_json::Value;

use super::tasks::acted_on;
use super::{Parameter, Served, required, take};
use crate::application::swarm::dto::{
    BlockTaskRequest, SubmitTaskRequest, TaskChange, TaskTransition, UnblockTaskRequest,
    VerifyTaskRequest,
};
use crate::application::swarm::use_cases::{BlockTask, SubmitTask, UnblockTask, VerifyTask};
use crate::domain::swarm::BoardError;

/// `block(task_id, token, reason)` and `unblock(task_id, token, reason)`.
pub(super) const BLOCK: [Parameter; 3] =
    [required("task_id"), required("token"), required("reason")];
/// `submit(task_id, token, evidence)`.
pub(super) const SUBMIT: [Parameter; 3] =
    [required("task_id"), required("token"), required("evidence")];
/// `verify_task(task_id, token, revision)`.
pub(super) const VERIFY_TASK: [Parameter; 3] =
    [required("task_id"), required("token"), required("revision")];

/// `None`, with `applied` as the decision of a change and `unchanged` for
/// a call that found the task already as asked, recording the task the
/// row holds (#2303).
fn answered(change: TaskChange, applied: &'static str) -> Served {
    Served {
        value: Value::Null,
        decision: match change.transition {
            TaskTransition::Applied => applied,
            TaskTransition::Unchanged => "unchanged",
        },
        task_id: acted_on(Some(&change.task_id)),
        message_id: None,
        cursor_moved: None,
    }
}

pub(super) fn block(
    block_task: &BlockTask,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [task_id, token, reason] = take(arguments)?;
    let change = block_task.execute(BlockTaskRequest {
        actor: actor.to_owned(),
        task_id,
        token,
        reason,
    })?;
    Ok(answered(change, "blocked"))
}

pub(super) fn unblock(
    unblock_task: &UnblockTask,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [task_id, token, reason] = take(arguments)?;
    let change = unblock_task.execute(UnblockTaskRequest {
        actor: actor.to_owned(),
        task_id,
        token,
        reason,
    })?;
    Ok(answered(change, "unblocked"))
}

pub(super) fn submit(
    submit_task: &SubmitTask,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [task_id, token, evidence] = take(arguments)?;
    let change = submit_task.execute(SubmitTaskRequest {
        actor: actor.to_owned(),
        task_id,
        token,
        evidence,
    })?;
    Ok(answered(change, "submitted"))
}

pub(super) fn verify_task(
    verify_task: &VerifyTask,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [task_id, token, revision] = take(arguments)?;
    let change = verify_task.execute(VerifyTaskRequest {
        actor: actor.to_owned(),
        task_id,
        token,
        revision,
    })?;
    Ok(answered(change, "verified"))
}

#[cfg(test)]
#[path = "swarm_board_dispatch_submissions_tests.rs"]
mod tests;
