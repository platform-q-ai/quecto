//! `Tasks.task_create` (#2272): a task is created once per request id.
use std::sync::Arc;

use serde_json::Value;

use super::OverRepository;
use crate::application::swarm::board_operation::{detail, operation};
use crate::application::swarm::board_tasks::read_task;
use crate::application::swarm::dto::{CreateTaskRequest, CreatedTask, NewTask};
use crate::application::swarm::ports::{BoardEncoding, BoardRepository, BoardTransaction, Clock};
use crate::domain::swarm::validation::{TEXT_MAX_BYTES, has_content};
use crate::domain::swarm::{
    Access, BoardError, RefusalKind, bounded, bounded_text, dependency_list, python_truthy,
    validate_dependencies,
};

/// The longest title, in UTF-8 bytes.
pub const TITLE_MAX_BYTES: usize = 1024;
/// The longest request id, in UTF-8 bytes (`Store.retry`).
pub const REQUEST_ID_MAX_BYTES: usize = 128;
/// The most tasks a board holds.
pub const TASK_BOARD_CAPACITY: i64 = 1000;

/// The title, then the acceptance list (a nonempty list of nonblank text,
/// its encoding bounded) are checked before the operation gate; falsy
/// dependencies are none (`dependencies or []`). Inside the gate (a
/// running run) the request id is bounded and the ledger replays a request
/// seen before, keyed by `['task', title, acceptance, dependencies]`. A new
/// request is refused on a full board, then inserts the task and only then
/// validates its dependencies against the new id, so an invalid list rolls
/// the insert back; the event `task_created{task}` follows, and the task's
/// dict is the answer the ledger stores.
pub struct CreateTask {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
    encoding: Arc<dyn BoardEncoding>,
}

impl CreateTask {
    pub fn new(
        repository: Arc<dyn BoardRepository>,
        clock: Arc<dyn Clock + Send + Sync>,
        encoding: Arc<dyn BoardEncoding>,
    ) -> Self {
        Self {
            repository,
            clock,
            encoding,
        }
    }

    /// # Errors
    /// An argument refusal, an authorisation or budget refusal, `request id
    /// reused with different payload`, the ledger's or the board's
    /// capacity, a dependency refusal, or the store's.
    pub fn execute(&self, request: CreateTaskRequest) -> Result<CreatedTask, BoardError> {
        let title = bounded(&request.title, "title", TITLE_MAX_BYTES)?;
        acceptance(&request.acceptance)?;
        bounded_text(
            &self.encoding.encode(&request.acceptance)?,
            "acceptance",
            TEXT_MAX_BYTES,
        )?;
        let dependencies = if python_truthy(&request.dependencies) {
            request.dependencies.clone()
        } else {
            Value::Array(Vec::new())
        };
        let payload = Value::Array(vec![
            Value::from("task"),
            Value::from(title),
            request.acceptance.clone(),
            dependencies.clone(),
        ]);
        let actor = request.actor.as_str();
        let running = Access {
            active: true,
            ..Access::default()
        };
        operation(
            &*self.repository,
            &*self.clock,
            actor,
            running,
            |transaction, _| {
                let id = bounded(&request.request, "request id", REQUEST_ID_MAX_BYTES)?;
                let mut created = false;
                let task = transaction.retry(actor, id, &payload, &mut || {
                    created = true;
                    let task = NewTask {
                        title: title.to_owned(),
                        acceptance: request.acceptance.clone(),
                        dependencies: dependencies.clone(),
                    };
                    self.create(transaction, actor, &task)
                })?;
                Ok(CreatedTask {
                    task,
                    replayed: !created,
                })
            },
        )
    }

    /// The ledger's action: the task's dict.
    fn create(
        &self,
        transaction: &dyn BoardTransaction,
        actor: &str,
        task: &NewTask,
    ) -> Result<Value, BoardError> {
        let held = transaction.task_count()?;
        if held >= TASK_BOARD_CAPACITY {
            return Err(BoardError::new(
                RefusalKind::CapacityFull,
                format!("task board full ({TASK_BOARD_CAPACITY}); settle existing work"),
            ));
        }
        let inserted = transaction.insert_task(task)?;
        let id = Value::from(inserted);
        let dependencies = dependency_list(&task.dependencies)?;
        validate_dependencies(&id, dependencies, &transaction.all_task_dependencies()?)?;
        transaction.event(
            actor,
            self.clock.now_seconds(),
            "task_created",
            &detail([("task", id.clone())]),
        )?;
        let created = read_task(transaction, &id)?;
        debug_assert!(
            created.get("id") == Some(&id)
                && created
                    .text("status")
                    .is_some_and(|status| ["ready", "blocked"].contains(&status)),
            "the new task reads back under its id, ready or blocked by its dependencies"
        );
        Ok(created.into_value())
    }
}

/// A nonempty list of text, each with content by Python's `str.strip`.
fn acceptance(value: &Value) -> Result<(), BoardError> {
    let criterion = |entry: &Value| entry.as_str().is_some_and(has_content);
    match value {
        Value::Array(entries) if !entries.is_empty() && entries.iter().all(criterion) => Ok(()),
        _ => Err(BoardError::new(
            RefusalKind::Invalid,
            "task acceptance criteria required: use a nonempty list[str], e.g. ['tests pass']",
        )),
    }
}

impl OverRepository for CreateTask {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
            encoding: self.encoding.clone(),
        }
    }
}

#[cfg(test)]
#[path = "create_task_tests.rs"]
mod tests;
