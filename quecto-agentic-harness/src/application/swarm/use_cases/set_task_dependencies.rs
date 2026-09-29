//! `Tasks.dependencies` (#2272): an unclaimed task's dependencies change.
use std::sync::Arc;

use serde_json::Value;

use crate::application::swarm::board_operation::{detail, operation};
use crate::application::swarm::board_tasks::read_task;
use crate::application::swarm::dto::SetTaskDependenciesRequest;
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{Access, BoardError, dependency_list, validate_dependencies};

/// Through the operation gate (a running run): only a task without an
/// owner whose status reads `ready` or `blocked` (a derived block
/// included) may change, and only to a bounded list of existing tasks,
/// never itself, closing no cycle. The list is stored as given, and the
/// event `dependencies{task,dependencies}` names the task id as the caller
/// gave it.
pub struct SetTaskDependencies {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl SetTaskDependencies {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// An authorisation or budget refusal, `unknown task`, `dependencies
    /// may change only before claiming`, a dependency refusal, or the
    /// store's.
    pub fn execute(&self, request: SetTaskDependenciesRequest) -> Result<(), BoardError> {
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
                let task = read_task(transaction, &request.task_id)?;
                let unowned = task.get("owner").is_some_and(Value::is_null);
                let unclaimed = matches!(task.text("status"), Some("ready" | "blocked"));
                match (unowned, unclaimed) {
                    (true, true) => {}
                    _ => {
                        return Err(BoardError::new(
                            "dependencies may change only before claiming",
                        ));
                    }
                }
                let dependencies = dependency_list(&request.dependencies)?;
                validate_dependencies(
                    &request.task_id,
                    dependencies,
                    &transaction.all_task_dependencies()?,
                )?;
                transaction.set_task_dependencies(&request.task_id, &request.dependencies)?;
                transaction.event(
                    actor,
                    self.clock.now_seconds(),
                    "dependencies",
                    &detail([
                        ("task", request.task_id.clone()),
                        ("dependencies", request.dependencies.clone()),
                    ]),
                )
            },
        )
    }
}

#[cfg(test)]
#[path = "set_task_dependencies_tests.rs"]
mod tests;
