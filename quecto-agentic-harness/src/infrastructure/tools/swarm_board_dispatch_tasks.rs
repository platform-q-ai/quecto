//! The task and claim methods of the board dispatch (#2272): their Python
//! signatures and their serving. Every argument reaches the use case as
//! the JSON value passed: Python binds a task id, a token or a request id
//! untyped and type-checks the rest at run time, so no type is refused
//! here.
use serde_json::Value;

use super::{Parameter, Served, SwarmBoardHandles, required, take};
#[cfg(any(test, feature = "test-support"))]
use crate::application::swarm::dto::ReadTaskRequest;
use crate::application::swarm::dto::{
    ClaimTaskRequest, CreateTaskRequest, ReleaseTaskRequest, SetTaskDependenciesRequest,
};
use crate::domain::swarm::BoardError;

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
/// `claim(task_id)`, and the test-only `task_raw(task_id)`.
pub(super) const CLAIM: [Parameter; 1] = [required("task_id")];
/// `release(task_id, token)`.
pub(super) const RELEASE: [Parameter; 2] = [required("task_id"), required("token")];

/// The task's dict, or the stored result of a retried request.
pub(super) fn task_create(
    handles: &SwarmBoardHandles,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [request, title, acceptance, dependencies] = take(arguments)?;
    let created = handles.create_task.execute(CreateTaskRequest {
        actor: actor.to_owned(),
        request,
        title,
        acceptance,
        dependencies,
    })?;
    Ok(Served {
        value: created.task,
        decision: if created.replayed {
            "replayed"
        } else {
            "created"
        },
    })
}

pub(super) fn dependencies(
    handles: &SwarmBoardHandles,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [task_id, dependencies] = take(arguments)?;
    handles
        .set_task_dependencies
        .execute(SetTaskDependenciesRequest {
            actor: actor.to_owned(),
            task_id,
            dependencies,
        })?;
    Ok(Served {
        value: Value::Null,
        decision: "updated",
    })
}

/// The claimed task's dict, its token included.
pub(super) fn claim(
    handles: &SwarmBoardHandles,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [task_id] = take(arguments)?;
    let task = handles.claim_task.execute(ClaimTaskRequest {
        actor: actor.to_owned(),
        task_id,
    })?;
    Ok(Served {
        value: task.into_value(),
        decision: "claimed",
    })
}

pub(super) fn release(
    handles: &SwarmBoardHandles,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [task_id, token] = take(arguments)?;
    handles.release_task.execute(ReleaseTaskRequest {
        actor: actor.to_owned(),
        task_id,
        token,
    })?;
    Ok(Served {
        value: Value::Null,
        decision: "released",
    })
}

/// `Tasks._task(db, task_id)` inside a read-only operation: the task's
/// dict without owner liveness.
#[cfg(any(test, feature = "test-support"))]
pub(super) fn task_raw(
    handles: &SwarmBoardHandles,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [task_id] = take(arguments)?;
    let task = handles.read_task.execute(ReadTaskRequest {
        actor: actor.to_owned(),
        task_id,
    })?;
    Ok(Served {
        value: task.into_value(),
        decision: "read",
    })
}

#[cfg(test)]
#[path = "swarm_board_dispatch_tasks_tests.rs"]
mod tests;
