//! Blockers, submissions and verification (#2272): the requests and
//! answers of `BlockTask`, `UnblockTask`, `SubmitTask` and `VerifyTask`.
//!
//! As in [`super::tasks`], `actor` is the member the call acts as and every
//! other argument stays the JSON value the caller passed: Python binds a
//! task id or a token untyped, writes the task id and a revision into
//! events as given, and compares stored values with them by Python's `==`.
use serde_json::Value;

use super::TaskRow;

/// Whether an owner operation changed the task or found it already as
/// asked: `block` with the stored blocker, `unblock` of a claimed task,
/// `submit` of the stored evidence, `verify_task` of completed work. Each
/// no-op answers as the change does and records no event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskTransition {
    Applied,
    Unchanged,
}

/// What an owner operation did, and the task's row as it now stands
/// (#2394: the op's answer, read in the op's own transaction). Its `id` is
/// the one the row holds, which the caller's id (`"2"`, `true`) only binds
/// to, for the op's telemetry record (#2303).
#[derive(Clone, Debug, PartialEq)]
pub struct TaskChange {
    pub task: TaskRow,
    pub transition: TaskTransition,
}

/// `Tasks.block(task_id, token, reason)` as `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockTaskRequest {
    pub actor: String,
    pub task_id: Value,
    pub token: Value,
    pub reason: Value,
}

/// `Tasks.unblock(task_id, token, reason)` as `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnblockTaskRequest {
    pub actor: String,
    pub task_id: Value,
    pub token: Value,
    pub reason: Value,
}

/// `Tasks.submit(task_id, token, evidence)` as `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubmitTaskRequest {
    pub actor: String,
    pub task_id: Value,
    pub token: Value,
    pub evidence: Value,
}

/// `Tasks.verify_task(task_id, token, revision)` as `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyTaskRequest {
    pub actor: String,
    pub task_id: Value,
    pub token: Value,
    pub revision: Value,
}
