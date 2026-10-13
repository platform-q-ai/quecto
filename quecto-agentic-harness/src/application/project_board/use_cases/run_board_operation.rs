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
use crate::application::project_board::ports::{BoardBootstrap, ReferenceStatus};
use crate::application::project_board::telemetry::{OpFacts, completion_facts_of};
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::telemetry::CompletionFacts;
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
        self.execute_noted(request, None)
    }

    /// [`Self::execute`], noting in `facts` (epic #2481 T4), when given,
    /// what its use case saw and, after a change that may complete its
    /// task, what came of completing it, with the completion's own write
    /// counted apart from the change's. Given none, nothing is noted or
    /// timed.
    ///
    /// # Errors
    /// As [`Self::execute`].
    pub fn execute_noted(
        &self,
        request: &BoardRequest,
        mut facts: Option<&mut OpFacts>,
    ) -> Result<BoardOutcome, ProjectBoardError> {
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
            BoardRequest::Show(id) => {
                let shown = self.parts.query.show(id)?;
                if let Some(facts) = facts {
                    let count = |items: usize| u64::try_from(items).unwrap_or(u64::MAX);
                    facts.plans = Some(count(shown.plans.len()));
                    facts.runs = Some(count(shown.runs.len()));
                }
                Ok(BoardOutcome::Shown(shown))
            }
            BoardRequest::Create(new) => {
                Ok(changed(self.parts.create.execute_noted(new, facts)?, None))
            }
            BoardRequest::UpdateDraft(update) => Ok(changed(
                self.parts.update.execute_noted(update, facts)?,
                None,
            )),
            BoardRequest::Change(id, op) => {
                let task = self
                    .parts
                    .change
                    .execute_noted(id, op, facts.as_deref_mut())?;
                let links = !task.fields().prs.is_empty();
                // The completion's own write is noted apart (only while
                // noting), so the change's attempts stay its own.
                let mut own = facts.as_ref().map(|_| OpFacts::default());
                let completion =
                    (completes(op) && links).then(|| completion(self.complete(id, own.as_mut())));
                if let (Some(facts), Some(completion)) = (facts, &completion) {
                    let own = own.unwrap_or_default();
                    facts.completion = Some(CompletionFacts {
                        completion_attempts: own.attempts,
                        completion_lost: own.lost,
                        completion_pause_ms: own.pause_ms,
                        ..completion_facts_of(completion, task.fields().prs.len())
                    });
                }
                Ok(changed(task, completion))
            }
            BoardRequest::Complete(id) => Ok(changed(self.complete(id, facts)?, None)),
            BoardRequest::Clear {
                path,
                object,
                repeated,
            } => Ok(BoardOutcome::Cleared {
                head: self
                    .parts
                    .clear
                    .execute_noted(path, object, repeated, facts)?,
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

    /// Task `id` completed, noted in `facts` when given; with no pull
    /// request host, a task not on the board is answered as such first.
    fn complete(&self, id: &Slug, facts: Option<&mut OpFacts>) -> Result<Task, ProjectBoardError> {
        match &self.parts.complete {
            Ok(complete) => complete.execute_noted(id, facts),
            Err(why) => {
                self.parts.query.show(id)?;
                Err(ProjectBoardError::NoPullRequestHost(why.clone()))
            }
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
