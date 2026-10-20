//! The coordination board's tasks.

use crate::application::swarm::dto::{CountedTask, NewTask, TaskRow, TaskUpdate};
use crate::domain::swarm::value_objects::board_error::BoardError;
use serde_json::Value;

/// The `tasks` rows (#2272). A task id is the caller's value, bound as
/// Python's `sqlite3` binds it (`"3"` finds task 3 through the column's
/// INTEGER affinity, `true` finds task 1, NULL finds nothing); a value it
/// cannot bind is refused naming its parameter.
pub trait BoardTasks {
    /// `SELECT * FROM tasks WHERE id=?` as `dict(row)`, with `acceptance`,
    /// `dependencies` and `evidence` loaded from their JSON (`_task`
    /// before it derives a blocked status).
    fn task(&self, id: &Value) -> Result<Option<TaskRow>, BoardError>;
    /// `SELECT status FROM tasks WHERE id=?`: the text, `None` for a
    /// missing row or a status that is not text.
    fn task_status(&self, id: &Value) -> Result<Option<String>, BoardError>;
    /// `SELECT * FROM tasks`: every task's id and its loaded dependencies,
    /// in store order (`_dependencies`' graph).
    fn all_task_dependencies(&self) -> Result<Vec<(i64, Value)>, BoardError>;
    fn task_count(&self) -> Result<i64, BoardError>;
    /// A new `ready` task with no evidence; its id.
    fn insert_task(&self, task: &NewTask) -> Result<i64, BoardError>;
    /// The task's dependencies, stored with the board's `encode()`. This and
    /// each task update after it change the one task `id` finds, which the
    /// caller has read in this transaction (the adapter asserts it).
    fn set_task_dependencies(&self, id: &Value, dependencies: &Value) -> Result<(), BoardError>;
    /// The task is `claimed` by `owner` under `token`.
    fn update_task_claim(&self, id: &Value, owner: &str, token: &str) -> Result<(), BoardError>;
    /// The task's status changes as `update` says.
    fn update_task_status(&self, id: &Value, update: &TaskUpdate) -> Result<(), BoardError>;
    /// `UPDATE tasks SET status='blocked',blocker=? WHERE owner=? AND
    /// status IN ('claimed','blocked','submitted')` (#1961, #2277): every
    /// active task of a member whose death is confirmed, the owner bound
    /// as given.
    fn block_owned_tasks(&self, owner: &Value, blocker: &str) -> Result<(), BoardError>;
    /// `SELECT id FROM tasks ORDER BY id LIMIT ? OFFSET ?` (#2277): the
    /// ids of a page, as stored. An offset beyond SQLite's integers is
    /// refused as the store refuses an integer it cannot bind.
    fn task_ids(&self, offset: u64, limit: i64) -> Result<Vec<Value>, BoardError>;
    /// `SELECT id,status,dependencies FROM tasks` (#2277), in store order,
    /// the dependencies loaded from their JSON.
    fn task_states(&self) -> Result<Vec<CountedTask>, BoardError>;
    /// `SELECT id, owner, status FROM tasks WHERE owner IS NOT NULL AND
    /// status IN ('claimed','blocked','submitted')` (#1969, #2277): the
    /// owner of each held claim, one per task, in store order.
    fn claim_owners(&self) -> Result<Vec<Value>, BoardError>;
}
