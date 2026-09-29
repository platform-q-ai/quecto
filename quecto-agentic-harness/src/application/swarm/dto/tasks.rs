//! Tasks and claims (#2272): the requests and results of `CreateTask`,
//! `SetTaskDependencies`, `ClaimTask`, `ReleaseTask` and `ReadTask`, and
//! the rows the board ports read and write for them.
//!
//! `actor` is the member the call acts as (Python's `store.actor`). Every
//! other argument stays the JSON value the caller passed: Python binds a
//! task id or a token to its SQL untyped (a `"3"` finds task 3 through the
//! column's INTEGER affinity, `true` finds task 1), writes it into the
//! event as given, and compares stored values with it by Python's `==`.
use serde_json::{Map, Value};

/// `dict(row)` of a `SELECT * FROM tasks` row, as `Tasks._task` returns
/// it: every column in table order, with `acceptance`, `dependencies` and
/// `evidence` loaded from their JSON, and a `ready` task whose
/// dependencies are not all completed reading `blocked` with the blocker
/// `unmet dependencies`.
#[derive(Clone, Debug, PartialEq)]
pub struct TaskRow {
    pub columns: Vec<(String, Value)>,
}

impl TaskRow {
    /// The value in `column`, when the row has that column.
    pub fn get(&self, column: &str) -> Option<&Value> {
        self.columns
            .iter()
            .find(|(name, _)| name == column)
            .map(|(_, value)| value)
    }

    /// The text in `column`: `None` for a missing column, NULL, or a value
    /// that is not text.
    pub fn text(&self, column: &str) -> Option<&str> {
        self.get(column).and_then(Value::as_str)
    }

    /// `result[column] = value`: a column the row has keeps its place, a
    /// new one goes last, as a Python dict does.
    pub fn set(&mut self, column: &str, value: Value) {
        match self.columns.iter_mut().find(|(name, _)| name == column) {
            Some(slot) => slot.1 = value,
            None => self.columns.push((column.to_owned(), value)),
        }
    }

    /// The task's dependencies as loaded: the stored list, or no entries
    /// for a column holding anything else.
    pub fn dependencies(&self) -> &[Value] {
        self.get("dependencies")
            .and_then(Value::as_array)
            .map_or(&[], Vec::as_slice)
    }

    /// The dict Python returns, key order included.
    pub fn into_value(self) -> Value {
        Value::Object(self.columns.into_iter().collect::<Map<String, Value>>())
    }
}

/// A new `ready` task, as `task_create` inserts it: the title, and the
/// acceptance list and dependencies the caller gave, each stored with the
/// board's `encode()`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewTask {
    pub title: String,
    pub acceptance: Value,
    pub dependencies: Value,
}

/// A change of a task's claim columns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskUpdate {
    /// `release`: the task is `ready` again, without owner, token or
    /// blocker.
    Release,
    /// `block`: the task is `blocked` by `reason`.
    Block { reason: String },
    /// `unblock`: the task is `claimed` again, without blocker.
    Unblock,
    /// `submit`: the task is `submitted` with `evidence` (stored with the
    /// board's `encode()`), without blocker.
    Submit { evidence: Value },
    /// `verify_task`: the task is `completed`.
    Complete,
    /// `recover` and `revoke` (#2275): the task is `ready` again, without
    /// owner, token or blocker, and its evidence is `[]`.
    Reopen,
}

/// `Tasks.task_create(request, title, acceptance, dependencies=None)` as
/// `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateTaskRequest {
    pub actor: String,
    pub request: Value,
    pub title: Value,
    pub acceptance: Value,
    pub dependencies: Value,
}

/// What `task_create` answered: the created task's dict, or the result
/// the request ledger stored for this request id (`replayed`), which is
/// the dict as `json.loads` reads the stored text back (keys sorted).
#[derive(Clone, Debug, PartialEq)]
pub struct CreatedTask {
    pub task: Value,
    pub replayed: bool,
}

/// `Tasks.dependencies(task_id, dependencies)` as `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetTaskDependenciesRequest {
    pub actor: String,
    pub task_id: Value,
    pub dependencies: Value,
}

/// `Tasks.claim(task_id)` as `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimTaskRequest {
    pub actor: String,
    pub task_id: Value,
}

/// `Tasks.release(task_id, token)` as `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseTaskRequest {
    pub actor: String,
    pub task_id: Value,
    pub token: Value,
}

/// `Tasks._task(db, task_id)` inside a read-only operation as `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadTaskRequest {
    pub actor: String,
    pub task_id: Value,
}
