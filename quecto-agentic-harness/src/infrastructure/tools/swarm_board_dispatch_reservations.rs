//! The file reservation, recovery and revocation methods of the board
//! dispatch (#2275): `reserve`, `release_files`, `file_owners`, `recover`
//! and `revoke`, with their Python signatures and their serving. Every
//! argument reaches the use case as the JSON value passed: Python binds a
//! task id, a token or a reservation untyped and type-checks the rest at
//! run time (`recover`'s `release_files` is compared by `is True`), so no
//! type is refused here. A record names the task by the id its row holds
//! (#2303), as the task methods' records do.
use serde_json::{Map, Value};

use super::tasks::acted_on;
use super::{Parameter, Served, required, take};
use crate::application::swarm::dto::{
    ListFileOwnersRequest, RecoverTaskRequest, ReleaseFilesRequest, ReserveFilesRequest,
    Revocation, RevokeTaskRequest,
};
use crate::application::swarm::use_cases::{
    ListFileOwners, RecoverTask, ReleaseFiles, ReserveFiles, RevokeTask,
};
use crate::domain::swarm::{BoardError, BoardOpDetail};

/// `reserve(task_id, token, paths)`.
pub(super) const RESERVE: [Parameter; 3] =
    [required("task_id"), required("token"), required("paths")];
/// `release_files(task_id, token, reservation)`.
pub(super) const RELEASE_FILES: [Parameter; 3] = [
    required("task_id"),
    required("token"),
    required("reservation"),
];
/// `file_owners(offset=0, limit=50)`.
pub(super) const FILE_OWNERS: [Parameter; 2] = [
    Parameter {
        name: "offset",
        default: Some(|| Value::from(0)),
    },
    Parameter {
        name: "limit",
        default: Some(|| Value::from(50)),
    },
];
/// `recover(task_id, release_files=False)`.
pub(super) const RECOVER: [Parameter; 2] = [
    required("task_id"),
    Parameter {
        name: "release_files",
        default: Some(|| Value::Bool(false)),
    },
];
/// `revoke(task_id, reason)`.
pub(super) const REVOKE: [Parameter; 2] = [required("task_id"), required("reason")];

/// `value` with `decision`, recording the task `task_id` (its row's id).
fn on_task(value: Value, decision: &'static str, task_id: &Value) -> Served {
    Served {
        value,
        decision,
        task_id: acted_on(Some(task_id)),
        message_id: None,
        cursor_moved: None,
        detail: BoardOpDetail::NONE,
        refused: None,
        controls_run: false,
    }
}

/// `{token, paths}`: the ownership token and the sorted paths.
pub(super) fn reserve(
    reserve_files: &ReserveFiles,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [task_id, token, paths] = take(arguments)?;
    let reservation = reserve_files.execute(ReserveFilesRequest {
        actor: actor.to_owned(),
        task_id,
        token,
        paths,
    })?;
    let mut answer = Map::new();
    answer.insert("token".to_owned(), Value::from(reservation.token));
    answer.insert("paths".to_owned(), Value::from(reservation.paths));
    Ok(on_task(
        Value::Object(answer),
        "reserved",
        &reservation.task_id,
    ))
}

pub(super) fn release_files(
    release_files: &ReleaseFiles,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [task_id, token, reservation] = take(arguments)?;
    let task_id = release_files.execute(ReleaseFilesRequest {
        actor: actor.to_owned(),
        task_id,
        token,
        reservation,
    })?;
    Ok(on_task(Value::Null, "released", &task_id))
}

/// The page of `files` rows, each as `dict(row)`.
pub(super) fn file_owners(
    list_file_owners: &ListFileOwners,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [offset, limit] = take(arguments)?;
    let rows = list_file_owners.execute(ListFileOwnersRequest {
        actor: actor.to_owned(),
        offset,
        limit,
    })?;
    Ok(Served {
        value: Value::Array(rows.into_iter().map(|row| row.into_value()).collect()),
        decision: "read",
        task_id: None,
        message_id: None,
        cursor_moved: None,
        detail: BoardOpDetail::NONE,
        refused: None,
        controls_run: false,
    })
}

pub(super) fn recover(
    recover_task: &RecoverTask,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [task_id, release_files] = take(arguments)?;
    let recovered = recover_task.execute(RecoverTaskRequest {
        actor: actor.to_owned(),
        task_id,
        release_files,
    })?;
    let decision = if recovered.reservations_released > 0 {
        "recovered_releasing_files"
    } else {
        "recovered"
    };
    Ok(on_task(Value::Null, decision, &recovered.task_id))
}

/// The task's dict as it now stands.
pub(super) fn revoke(
    revoke_task: &RevokeTask,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [task_id, reason] = take(arguments)?;
    let revoked = revoke_task.execute(RevokeTaskRequest {
        actor: actor.to_owned(),
        task_id,
        reason,
    })?;
    let decision = match revoked.revocation {
        Revocation::Unowned => "unchanged",
        Revocation::Revoked { notified: true } => "revoked_notified",
        Revocation::Revoked { notified: false } => "revoked_unnotified",
    };
    let stored = revoked.task.get("id").cloned().unwrap_or(Value::Null);
    Ok(on_task(revoked.task.into_value(), decision, &stored))
}

#[cfg(test)]
#[path = "swarm_board_dispatch_reservations_tests.rs"]
mod tests;
