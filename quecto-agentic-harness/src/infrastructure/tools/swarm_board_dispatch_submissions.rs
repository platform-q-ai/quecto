//! The blocker, submission and verification methods of the board dispatch
//! (#2272): their Python signatures and their serving. Every argument
//! reaches the use case as the JSON value passed (Python binds a task id
//! or a token untyped and type-checks the rest at run time), and each
//! method answers `None`. The decision names whether the call changed the
//! task or found it already as asked.
use serde_json::Value;

use super::{Parameter, Served, required, take};
use crate::application::swarm::dto::{
    BlockTaskRequest, SubmitTaskRequest, TaskTransition, UnblockTaskRequest, VerifyTaskRequest,
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
/// a call that found the task already as asked.
fn answered(transition: TaskTransition, applied: &'static str) -> Served {
    Served {
        value: Value::Null,
        decision: match transition {
            TaskTransition::Applied => applied,
            TaskTransition::Unchanged => "unchanged",
        },
        task_id: None,
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
    let transition = block_task.execute(BlockTaskRequest {
        actor: actor.to_owned(),
        task_id,
        token,
        reason,
    })?;
    Ok(answered(transition, "blocked"))
}

pub(super) fn unblock(
    unblock_task: &UnblockTask,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [task_id, token, reason] = take(arguments)?;
    let transition = unblock_task.execute(UnblockTaskRequest {
        actor: actor.to_owned(),
        task_id,
        token,
        reason,
    })?;
    Ok(answered(transition, "unblocked"))
}

pub(super) fn submit(
    submit_task: &SubmitTask,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [task_id, token, evidence] = take(arguments)?;
    let transition = submit_task.execute(SubmitTaskRequest {
        actor: actor.to_owned(),
        task_id,
        token,
        evidence,
    })?;
    Ok(answered(transition, "submitted"))
}

pub(super) fn verify_task(
    verify_task: &VerifyTask,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [task_id, token, revision] = take(arguments)?;
    let transition = verify_task.execute(VerifyTaskRequest {
        actor: actor.to_owned(),
        task_id,
        token,
        revision,
    })?;
    Ok(answered(transition, "verified"))
}

#[cfg(test)]
#[path = "swarm_board_dispatch_submissions_tests.rs"]
mod tests;
