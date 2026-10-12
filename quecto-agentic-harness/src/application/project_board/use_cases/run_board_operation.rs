//! Runs one typed board operation (#2477).
//!
//! - After every pull request linked and every move to review, a task
//!   that links pull requests is completed if they have all merged
//!   (`CompleteMergedTask`), and the outcome says what came of that: done,
//!   pending on the ones not merged yet, or failed.
//! - [`RunBoardOperation::clock_note`] says why a request that reads the
//!   clock was not bounded against an outside time, when it has none.
//!
//! Blocking: the store, the listings, the clock and the host all block, so
//! callers run operations off the async workers.
use crate::application::project_board::dto::{BoardListing, BoardOp, ProjectBoardError};
use crate::application::project_board::operations::{
    BoardOperationParts, BoardOutcome, BoardRequest, Completion, ListFilter,
};
use crate::application::project_board::ports::{ArtifactKey, BoardBootstrap, ReferenceStatus};
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;

pub struct RunBoardOperation {
    parts: BoardOperationParts,
}

impl RunBoardOperation {
    /// The operations over `parts`, as composition built them.
    pub fn new(parts: BoardOperationParts) -> Self {
        Self { parts }
    }

    /// What `request` comes to.
    ///
    /// # Errors
    /// The use case's or the listing's refusal.
    pub fn execute(&self, request: &BoardRequest) -> Result<BoardOutcome, ProjectBoardError> {
        let listing = |listing: Result<BoardListing, _>| {
            listing
                .map(BoardOutcome::Listing)
                .map_err(ProjectBoardError::from)
        };
        match request {
            BoardRequest::Bootstrap => Ok(match self.parts.bootstrap.execute()? {
                BoardBootstrap::Created(head) => BoardOutcome::Bootstrapped {
                    head,
                    created: true,
                },
                BoardBootstrap::Existing(head) => BoardOutcome::Bootstrapped {
                    head,
                    created: false,
                },
            }),
            BoardRequest::List(filter) => listing(match filter {
                ListFilter::All => self.parts.listings.all(),
                ListFilter::Status(status) => self.parts.listings.by_status(*status),
                ListFilter::Children(parent) => self.parts.listings.children(parent),
                ListFilter::Dependents(task) => self.parts.listings.dependents(task),
            }),
            BoardRequest::ListReady => listing(self.parts.listings.ready()),
            BoardRequest::ListTakeable => {
                let now = self.parts.clock.now()?;
                listing(self.parts.listings.takeable(&now))
            }
            BoardRequest::Show(id) => self.show(id),
            BoardRequest::Create(new) => Ok(changed(self.parts.create.execute(new)?, None)),
            BoardRequest::UpdateDraft(update) => {
                Ok(changed(self.parts.update.execute(update)?, None))
            }
            BoardRequest::Change(id, op) => {
                let task = self.parts.change.execute(id, op)?;
                let links = !task.fields().prs.is_empty();
                let completion = (completes(op) && links).then(|| completion(self.complete(id)));
                Ok(changed(task, completion))
            }
            BoardRequest::Complete(id) => Ok(changed(self.complete(id)?, None)),
            BoardRequest::Clear {
                path,
                object,
                repeated,
            } => Ok(BoardOutcome::Cleared {
                head: self.parts.clear.execute(path, object, repeated)?,
            }),
        }
    }

    /// Why a request that reads the clock was not bounded against an
    /// outside time, when the clock has no reference; `None` for any other
    /// request.
    pub fn clock_note(&self, request: &BoardRequest) -> Option<String> {
        match (request.reads_clock(), self.parts.clock.reference_status()) {
            (true, ReferenceStatus::NoReference(why)) => Some(why),
            (true, ReferenceStatus::Referenced) | (false, _) => None,
        }
    }

    /// Task `id` completed; with no pull request host, a task not on the
    /// board is answered as such first.
    fn complete(&self, id: &Slug) -> Result<Task, ProjectBoardError> {
        match &self.parts.complete {
            Ok(complete) => complete.execute(id),
            Err(why) => {
                self.show(id)?;
                Err(ProjectBoardError::NoPullRequestHost(why.clone()))
            }
        }
    }

    fn show(&self, id: &Slug) -> Result<BoardOutcome, ProjectBoardError> {
        let board = self.parts.listings.all()?;
        match board.tasks.iter().find(|task| task.fields().id == *id) {
            Some(task) => Ok(BoardOutcome::Shown {
                head: board.head.clone(),
                task: task.clone(),
            }),
            None if board.quarantined.iter().any(|file| file.id() == Some(id)) => Err(
                ProjectBoardError::Quarantined(ArtifactKey::Task(id.clone())),
            ),
            None => Err(ProjectBoardError::NotFound(id.clone())),
        }
    }
}

fn changed(task: Task, completion: Option<Completion>) -> BoardOutcome {
    BoardOutcome::Changed { task, completion }
}

/// A completion's result, typed: pull requests not merged yet are pending,
/// not an error.
fn completion(result: Result<Task, ProjectBoardError>) -> Completion {
    match result {
        Ok(task) => Completion::Done(Box::new(task)),
        Err(ProjectBoardError::Rule(BoardRuleError::PrsUnmerged(prs))) => Completion::Pending(prs),
        Err(error) => Completion::Failed(Box::new(error)),
    }
}

/// A change after which the task may be done: a pull request linked (it
/// may have merged before), or a move to review.
fn completes(op: &BoardOp) -> bool {
    matches!(op, BoardOp::AddPr(_) | BoardOp::Move(TaskStatus::Review))
}

#[cfg(test)]
#[path = "run_board_operation_tests.rs"]
mod tests;
