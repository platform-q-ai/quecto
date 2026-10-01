//! File reservations, recovery and revocation (#2275): the requests and
//! answers of `ReserveFiles`, `ReleaseFiles`, `ListFileOwners`,
//! `RecoverTask` and `RevokeTask`, and the rows the board ports read and
//! write for them.
//!
//! As in [`super::tasks`], `actor` is the member the call acts as and every
//! other argument stays the JSON value the caller passed: Python binds a
//! task id, a token or a reservation untyped, writes them into events as
//! given, and type-checks the rest at run time.
use serde_json::{Map, Value};

use super::TaskRow;

/// `Tasks.reserve(task_id, token, paths)` as `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReserveFilesRequest {
    pub actor: String,
    pub task_id: Value,
    pub token: Value,
    pub paths: Value,
}

/// What `reserve` answered: the ownership token drawn for the set, and
/// the normalised paths in sorted order; and the task's id as its row
/// holds it (#2303), for the call's record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reservation {
    pub task_id: Value,
    pub token: String,
    pub paths: Vec<String>,
}

/// One reservation set, as `reserve` inserts it: a row per path, each
/// naming the task and the claim as the caller gave them, the owner, and
/// the ownership token. The paths are distinct and sorted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewReservation {
    pub task: Value,
    pub owner: String,
    pub claim: Value,
    pub token: String,
    pub paths: Vec<String>,
}

/// `Tasks.release_files(task_id, token, reservation)` as `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseFilesRequest {
    pub actor: String,
    pub task_id: Value,
    pub token: Value,
    pub reservation: Value,
}

/// `Tasks.file_owners(offset=0, limit=50)` as `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListFileOwnersRequest {
    pub actor: String,
    pub offset: Value,
    pub limit: Value,
}

/// `dict(row)` of a `SELECT * FROM files` row: every column, in table
/// order, as stored.
#[derive(Clone, Debug, PartialEq)]
pub struct FileRow {
    pub columns: Vec<(String, Value)>,
}

impl FileRow {
    /// The dict Python returns, key order included.
    pub fn into_value(self) -> Value {
        Value::Object(self.columns.into_iter().collect::<Map<String, Value>>())
    }
}

/// `Tasks.recover(task_id, release_files=False)` as `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoverTaskRequest {
    pub actor: String,
    pub task_id: Value,
    pub release_files: Value,
}

/// What `release_files` did (#2394 round-1 review L1): the task's id as
/// its row holds it (#2303), and how many files the reservation released
/// (none for a reservation that holds nothing).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleasedFiles {
    pub task_id: Value,
    pub released: usize,
}

/// What `recover` did: the reservations it released with the task, and
/// the task's row as it now stands (#2394: the op answers it), whose id
/// the call's record names (#2303).
#[derive(Clone, Debug, PartialEq)]
pub struct Recovered {
    pub task: TaskRow,
    pub reservations_released: i64,
}

/// `Tasks.revoke(task_id, reason)` as `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RevokeTaskRequest {
    pub actor: String,
    pub task_id: Value,
    pub reason: Value,
}

/// What `revoke` answered: the task as it now stands, and what was done.
#[derive(Clone, Debug, PartialEq)]
pub struct Revoked {
    pub task: TaskRow,
    pub revocation: Revocation,
}

/// Whether `revoke` took a claim back, and whether the previous owner was
/// told.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Revocation {
    /// The task had no owner and was `ready` or `blocked`: nothing changed.
    Unowned,
    /// The claim was taken back; `notified` when the previous owner got a
    /// board message (not when it is dead, unknown, or its inbox is full).
    Revoked { notified: bool },
}
