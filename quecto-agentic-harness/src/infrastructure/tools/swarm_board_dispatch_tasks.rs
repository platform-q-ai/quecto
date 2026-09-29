//! The task and claim methods of the board dispatch (#2272): their Python
//! signatures and their serving. Every argument reaches the use case as
//! the JSON value passed: Python binds a task id, a token or a request id
//! untyped and type-checks the rest at run time, so no type is refused
//! here.
//!
//! Each records the task it acted on (#2303): the id of the task row the
//! board read, never the argument as given (a `"2"` or a `true` binds to
//! task 2 or 1 as SQLite's affinity finds it), and `None` for an id that
//! is not an integer (a row edited from outside). None moves a message
//! cursor or acts on a message.
use serde_json::Value;

use super::{Parameter, Served, required, take};
use crate::application::swarm::dto::{
    ClaimTaskRequest, CreateTaskRequest, ReadTaskRequest, ReleaseTaskRequest,
    SetTaskDependenciesRequest,
};
use crate::application::swarm::use_cases::{
    ClaimTask, CreateTask, ReadTask, ReleaseTask, SetTaskDependencies,
};
use crate::domain::swarm::{BoardError, BoardOpDetail};

/// `task_create(request, title, acceptance, dependencies=None)`.
pub(super) const TASK_CREATE: [Parameter; 4] = [
    required("request"),
    required("title"),
    required("acceptance"),
    Parameter {
        name: "dependencies",
        default: Some(|| Value::Null),
    },
];
/// `dependencies(task_id, dependencies)`.
pub(super) const DEPENDENCIES: [Parameter; 2] = [required("task_id"), required("dependencies")];
/// `claim(task_id)`, `task(task_id)`, and the test-only `task_raw(task_id)`.
pub(super) const CLAIM: [Parameter; 1] = [required("task_id")];
/// `release(task_id, token)`.
pub(super) const RELEASE: [Parameter; 2] = [required("task_id"), required("token")];

/// The task's dict, or the stored result of a retried request.
pub(super) fn task_create(
    create_task: &CreateTask,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [request, title, acceptance, dependencies] = take(arguments)?;
    let created = create_task.execute(CreateTaskRequest {
        actor: actor.to_owned(),
        request,
        title,
        acceptance,
        dependencies,
    })?;
    let task_id = acted_on(created.task.get("id"));
    Ok(Served {
        value: created.task,
        decision: if created.replayed {
            "replayed"
        } else {
            "created"
        },
        task_id,
        message_id: None,
        cursor_moved: None,
        detail: BoardOpDetail::NONE,
    })
}

pub(super) fn dependencies(
    set_task_dependencies: &SetTaskDependencies,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [task_id, dependencies] = take(arguments)?;
    let task = set_task_dependencies.execute(SetTaskDependenciesRequest {
        actor: actor.to_owned(),
        task_id,
        dependencies,
    })?;
    Ok(Served {
        value: Value::Null,
        decision: "updated",
        task_id: acted_on(Some(&task)),
        message_id: None,
        cursor_moved: None,
        detail: BoardOpDetail::NONE,
    })
}

/// The claimed task's dict, its token included.
pub(super) fn claim(
    claim_task: &ClaimTask,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [task_id] = take(arguments)?;
    let task = claim_task.execute(ClaimTaskRequest {
        actor: actor.to_owned(),
        task_id,
    })?;
    let task_id = acted_on(task.get("id"));
    Ok(Served {
        value: task.into_value(),
        decision: "claimed",
        task_id,
        message_id: None,
        cursor_moved: None,
        detail: BoardOpDetail::NONE,
    })
}

pub(super) fn release(
    release_task: &ReleaseTask,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [task_id, token] = take(arguments)?;
    let task = release_task.execute(ReleaseTaskRequest {
        actor: actor.to_owned(),
        task_id,
        token,
    })?;
    Ok(Served {
        value: Value::Null,
        decision: "released",
        task_id: acted_on(Some(&task)),
        message_id: None,
        cursor_moved: None,
        detail: BoardOpDetail::NONE,
    })
}

/// `task(task_id)`: the task's dict, with its owner's liveness for a
/// held claim (#2277).
pub(super) fn task(
    read_task: &ReadTask,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [task_id] = take(arguments)?;
    let task = read_task.execute(ReadTaskRequest {
        actor: actor.to_owned(),
        task_id,
    })?;
    let task_id = acted_on(task.get("id"));
    Ok(Served {
        value: task.into_value(),
        decision: "read",
        task_id,
        message_id: None,
        cursor_moved: None,
        detail: BoardOpDetail::NONE,
    })
}

/// The owner-liveness keys `task` adds to a task's dict (#1969).
#[cfg(any(test, feature = "test-support"))]
const LIVENESS: [&str; 4] = ["owner_last_activity", "owner_state", "contact", "recovery"];

/// The test-only `task_raw(task_id)`: `Tasks._task` as `task` reads it,
/// without the owner's liveness (the differential harness's alias).
#[cfg(any(test, feature = "test-support"))]
pub(super) fn task_raw(
    read_task: &ReadTask,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let mut served = task(read_task, actor, arguments)?;
    if let Value::Object(columns) = std::mem::take(&mut served.value) {
        let raw = columns
            .into_iter()
            .filter(|(column, _)| !LIVENESS.contains(&column.as_str()));
        served.value = Value::Object(raw.collect());
    }
    Ok(served)
}

/// The task an op acted on, for its records: the row's stored id when it
/// is an integer, as the board writes it.
pub(super) fn acted_on(id: Option<&Value>) -> Option<i64> {
    id.and_then(Value::as_i64)
}

#[cfg(test)]
#[path = "swarm_board_dispatch_tasks_tests.rs"]
mod tests;
