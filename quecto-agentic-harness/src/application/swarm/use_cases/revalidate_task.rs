//! `Workbench.revalidate_task(task_id, revision, evidence)` (#2273): the
//! coordinator gives completed work new evidence at a later revision, so
//! that the run can complete at it.
use std::sync::Arc;

use serde_json::Value;

use super::OverRepository;
use crate::application::swarm::board_completion::task_record;
use crate::application::swarm::board_operation::{detail, operation};
use crate::application::swarm::board_tasks::stored_id;
use crate::application::swarm::dto::RevalidateTaskRequest;
use crate::application::swarm::ports::{BoardEncoding, BoardRepository, Clock};
use crate::domain::swarm::validation::TEXT_MAX_BYTES;
use crate::domain::swarm::{Access, BoardError, RefusalKind, bounded_text, revalidation};

/// The evidence's encoding is bounded before the operation gate, which
/// admits only the coordinator (a running run). The task is read as
/// stored (`unknown task` when there is none) and the domain's
/// `revalidation` checks it is completed and that the evidence is a
/// nonempty list of `{artifact, revision}` at the revision. The evidence
/// then replaces the task's as given (plain `json.dumps`), and the event
/// `revalidated{task,revision,previous_evidence,evidence}` names the task
/// id and the revision as the caller gave them. The answer is the id the
/// task's row holds, for the op's telemetry record (#2303).
pub struct RevalidateTask {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
    encoding: Arc<dyn BoardEncoding>,
}

impl RevalidateTask {
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
    /// The evidence's bound, an authorisation or budget refusal, `unknown
    /// task`, a revalidation refusal with Python's text, or the store's.
    pub fn execute(&self, request: RevalidateTaskRequest) -> Result<Value, BoardError> {
        bounded_text(
            &self.encoding.encode(&request.evidence)?,
            "evidence references",
            TEXT_MAX_BYTES,
        )?;
        let actor = request.actor.as_str();
        let coordinating = Access {
            active: true,
            coordinator: true,
            ..Access::default()
        };
        operation(
            &*self.repository,
            &*self.clock,
            actor,
            coordinating,
            |transaction, _| {
                let Some(task) = transaction.task(&request.task_id)? else {
                    return Err(BoardError::new(RefusalKind::NotFound, "unknown task"));
                };
                let evidence =
                    revalidation(&task_record(&task), &request.revision, &request.evidence)?;
                let previous = task.get("evidence").cloned().ok_or_else(|| {
                    BoardError::new(RefusalKind::Store, "the board's task row has no evidence")
                })?;
                transaction.replace_task_evidence(&request.task_id, evidence)?;
                transaction.event(
                    actor,
                    self.clock.now_seconds(),
                    "revalidated",
                    &detail([
                        ("task", request.task_id.clone()),
                        ("revision", request.revision.clone()),
                        ("previous_evidence", previous),
                        ("evidence", evidence.clone()),
                    ]),
                )?;
                Ok(stored_id(&task))
            },
        )
    }
}

impl OverRepository for RevalidateTask {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
            encoding: self.encoding.clone(),
        }
    }
}

#[cfg(test)]
#[path = "revalidate_task_tests.rs"]
mod tests;
