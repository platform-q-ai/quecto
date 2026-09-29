//! Test support (compiled only under `cfg(test)`): the in-memory board's
//! tasks, request ledger and file reservations (#2272). Task ids match by a
//! rough INTEGER affinity (an integral number, a boolean, or numeric text);
//! the SQLite adapter's contract tests pin the real one.
use serde_json::{Value, json};

use super::MemoryTransaction;
use crate::application::swarm::dto::{NewTask, TaskRow, TaskUpdate};
use crate::application::swarm::ports::{BoardFiles, BoardRequests, BoardTasks, RequestAction};
use crate::domain::swarm::{BoardError, RefusalKind};

/// One `requests` row.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredRequest {
    pub actor: String,
    pub request: String,
    pub payload: Value,
    pub result: Value,
}

/// One `files` row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredFile {
    pub path: String,
    pub task: i64,
    pub owner: String,
    pub claim: String,
    pub token: String,
}

/// A `tasks` row in the board's column order, JSON columns loaded.
pub fn stored_task(id: i64, status: &str, dependencies: Value, owner: Option<&str>) -> TaskRow {
    let text = |value: Option<&str>| value.map_or(Value::Null, Value::from);
    TaskRow {
        columns: vec![
            ("id".to_owned(), json!(id)),
            ("title".to_owned(), json!(format!("task {id}"))),
            ("acceptance".to_owned(), json!(["pass"])),
            ("dependencies".to_owned(), dependencies),
            ("status".to_owned(), json!(status)),
            ("owner".to_owned(), text(owner)),
            ("token".to_owned(), text(owner.map(|_| "stored-token"))),
            ("evidence".to_owned(), json!([])),
            ("blocker".to_owned(), Value::Null),
        ],
    }
}

/// Roughly the integer an INTEGER PRIMARY KEY compares a bound value as.
fn affinity(value: &Value) -> Option<i64> {
    match value {
        Value::Bool(flag) => Some(i64::from(*flag)),
        Value::Number(number) => number.as_i64().or_else(|| {
            number
                .as_f64()
                .filter(|float| float.fract() == 0.0)
                .map(|float| float as i64)
        }),
        Value::String(text) => text.trim().parse().ok(),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

fn row_id(row: &TaskRow) -> Option<i64> {
    row.get("id").and_then(Value::as_i64)
}

impl MemoryTransaction<'_> {
    fn change_task(&self, id: &Value, change: impl Fn(&mut TaskRow)) {
        let wanted = affinity(id);
        let mut state = self.state.borrow_mut();
        for row in &mut state.tasks {
            if wanted.is_some() && row_id(row) == wanted {
                change(row);
            }
        }
    }
}

impl BoardTasks for MemoryTransaction<'_> {
    fn task(&self, id: &Value) -> Result<Option<TaskRow>, BoardError> {
        let wanted = affinity(id);
        Ok(self
            .state
            .borrow()
            .tasks
            .iter()
            .find(|row| wanted.is_some() && row_id(row) == wanted)
            .cloned())
    }

    fn task_status(&self, id: &Value) -> Result<Option<String>, BoardError> {
        Ok(self
            .task(id)?
            .and_then(|row| row.text("status").map(str::to_owned)))
    }

    fn all_task_dependencies(&self) -> Result<Vec<(i64, Value)>, BoardError> {
        self.note("all_task_dependencies".to_owned());
        Ok(self
            .state
            .borrow()
            .tasks
            .iter()
            .map(|row| {
                (
                    row_id(row).expect("a stored task has an id"),
                    row.get("dependencies").cloned().unwrap_or(Value::Null),
                )
            })
            .collect())
    }

    fn task_count(&self) -> Result<i64, BoardError> {
        Ok(i64::try_from(self.state.borrow().tasks.len()).unwrap())
    }

    fn insert_task(&self, task: &NewTask) -> Result<i64, BoardError> {
        let mut state = self.state.borrow_mut();
        let id = state.tasks.iter().filter_map(row_id).max().unwrap_or(0) + 1;
        self.note(format!("insert_task {id}"));
        let mut row = stored_task(id, "ready", task.dependencies.clone(), None);
        row.set("title", Value::from(task.title.clone()));
        row.set("acceptance", task.acceptance.clone());
        state.tasks.push(row);
        Ok(id)
    }

    fn set_task_dependencies(&self, id: &Value, dependencies: &Value) -> Result<(), BoardError> {
        self.note(format!("set_task_dependencies {id}"));
        self.change_task(id, |row| row.set("dependencies", dependencies.clone()));
        Ok(())
    }

    fn update_task_claim(&self, id: &Value, owner: &str, token: &str) -> Result<(), BoardError> {
        self.note(format!("update_task_claim {id} {owner}"));
        self.change_task(id, |row| {
            row.set("status", Value::from("claimed"));
            row.set("owner", Value::from(owner));
            row.set("token", Value::from(token));
        });
        Ok(())
    }

    fn update_task_status(&self, id: &Value, update: &TaskUpdate) -> Result<(), BoardError> {
        self.note(format!("update_task_status {id} {update:?}"));
        self.change_task(id, |row| match update {
            TaskUpdate::Release => {
                row.set("status", Value::from("ready"));
                row.set("owner", Value::Null);
                row.set("token", Value::Null);
                row.set("blocker", Value::Null);
            }
            TaskUpdate::Block { reason } => {
                row.set("status", Value::from("blocked"));
                row.set("blocker", Value::from(reason.clone()));
            }
            TaskUpdate::Unblock => {
                row.set("status", Value::from("claimed"));
                row.set("blocker", Value::Null);
            }
            TaskUpdate::Submit { evidence } => {
                row.set("status", Value::from("submitted"));
                row.set("evidence", evidence.clone());
                row.set("blocker", Value::Null);
            }
            TaskUpdate::Complete => row.set("status", Value::from("completed")),
        });
        Ok(())
    }
}

impl BoardRequests for MemoryTransaction<'_> {
    fn retry(
        &self,
        actor: &str,
        request: &str,
        payload: &Value,
        action: &mut RequestAction<'_>,
    ) -> Result<Value, BoardError> {
        let stored = self
            .state
            .borrow()
            .requests
            .iter()
            .find(|row| row.actor == actor && row.request == request)
            .cloned();
        match stored {
            Some(row) if row.payload == *payload => Ok(row.result),
            Some(_) => Err(BoardError::new(
                RefusalKind::RequestIdReused,
                "request id reused with different payload",
            )),
            None => {
                let result = action()?;
                self.note(format!("store_request {request}"));
                self.state.borrow_mut().requests.push(StoredRequest {
                    actor: actor.to_owned(),
                    request: request.to_owned(),
                    payload: payload.clone(),
                    result: result.clone(),
                });
                Ok(result)
            }
        }
    }
}

impl BoardFiles for MemoryTransaction<'_> {
    fn delete_claim_files(&self, task: &Value, claim: &Value) -> Result<(), BoardError> {
        self.note(format!("delete_claim_files {task}"));
        let task = affinity(task);
        self.state
            .borrow_mut()
            .files
            .retain(|file| !(Some(file.task) == task && claim.as_str() == Some(&file.claim)));
        Ok(())
    }
}
