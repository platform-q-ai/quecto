//! Runs one typed board operation (#2477).
//!
//! - After every pull request linked and every move to review, a task
//!   that links pull requests is completed if they have all merged
//!   (`CompleteMergedTask`), and the outcome says what came of that: done,
//!   pending on the ones not merged yet, or failed.
//! - A listing asked for summaries ([`ListSummaries::Wanted`]) has its
//!   tasks each summarised from one read of the board made after the
//!   listing; while the board moved in between, the listing is read
//!   again, a bounded number of times ([`MAX_LIST_ATTEMPTS`]). A listing
//!   is never refused for its summaries: when they cannot be read, it is
//!   answered without them, saying why. Asked for none, it reads only
//!   what it lists, as ever.
//! - [`RunBoardOperation::clock_note`] says why a request that reads the
//!   clock was not bounded against an outside time, when it has none.
//!
//! Blocking: the store, the listings, the clock and the host all block, so
//! callers run operations off the async workers.
use crate::application::project_board::dto::{BoardListing, BoardOp, ProjectBoardError};
use crate::application::project_board::operations::{
    BoardOperationParts, BoardOutcome, BoardRequest, Completion, ListFilter, ListSummaries,
};
use crate::application::project_board::ports::{BoardBootstrap, BoardStoreError, ReferenceStatus};
use crate::application::project_board::task_summary::{Listed, SkipReason, Summarized};
use crate::application::project_board::telemetry::{OpFacts, completion_facts_of};
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::telemetry::CompletionFacts;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;

/// Up to 3 reads for the summaries; each listing read again also rebuilds
/// the index (itself a whole read) when the board moved. Past that, the
/// summaries are skipped as contended.
pub const MAX_LIST_ATTEMPTS: usize = 3;

pub struct RunBoardOperation {
    parts: BoardOperationParts,
}

impl RunBoardOperation {
    /// The operations over `parts`, as composition built them.
    pub fn new(parts: BoardOperationParts) -> Self {
        Self { parts }
    }

    /// The ends this front end owes as its session ends (#2655): each run
    /// its starts launched that the board still shows open, to be ended as
    /// [`ListLeftRuns`](super::ListLeftRuns) says; none for a front end
    /// that launches none.
    ///
    /// # Errors
    /// The store's.
    pub fn left_runs(
        &self,
    ) -> Result<Vec<crate::application::project_board::dto::RunEnd>, ProjectBoardError> {
        match &self.parts.left_runs {
            Some(left) => left.execute(),
            None => Ok(vec![]),
        }
    }

    /// What `request` comes to.
    ///
    /// # Errors
    /// The use case's or the listing's refusal.
    pub fn execute(&self, request: &BoardRequest) -> Result<BoardOutcome, ProjectBoardError> {
        self.execute_noted(request, None)
    }

    /// [`Self::execute_noted`], a listing summarising its tasks as
    /// `summaries` asks.
    ///
    /// # Errors
    /// As [`Self::execute`]: never for a listing's summaries.
    ///
    /// # Panics
    /// Summaries asked of a request that is no listing
    /// ([`BoardRequest::is_listing`]): a front end refuses that first.
    pub fn execute_noted_with(
        &self,
        request: &BoardRequest,
        facts: Option<&mut OpFacts>,
        summaries: ListSummaries,
    ) -> Result<BoardOutcome, ProjectBoardError> {
        assert!(
            summaries == ListSummaries::Omitted || request.is_listing(),
            "summaries are asked only of a listing: {request:?}"
        );
        self.run(request, facts, summaries)
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
        facts: Option<&mut OpFacts>,
    ) -> Result<BoardOutcome, ProjectBoardError> {
        self.run(request, facts, ListSummaries::Omitted)
    }

    fn run(
        &self,
        request: &BoardRequest,
        mut facts: Option<&mut OpFacts>,
        summaries: ListSummaries,
    ) -> Result<BoardOutcome, ProjectBoardError> {
        let listings = &self.parts.listings;
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
            BoardRequest::List(filter) => self.listed(
                || match filter {
                    ListFilter::All => listings.all(),
                    ListFilter::Status(status) => listings.by_status(*status),
                    ListFilter::Children(parent) => listings.children(parent),
                    ListFilter::Dependents(task) => listings.dependents(task),
                },
                summaries,
            ),
            BoardRequest::ListReady => self.listed(|| listings.ready(), summaries),
            BoardRequest::ListTakeable => {
                let now = self.parts.clock.now()?;
                self.listed(|| listings.takeable(&now), summaries)
            }
            BoardRequest::Show(id, view) => {
                let shown = self.parts.query.show(id)?;
                if let Some(facts) = facts {
                    let count = |items: usize| u64::try_from(items).unwrap_or(u64::MAX);
                    facts.plans = Some(count(shown.plans.len()));
                    facts.runs = Some(count(shown.runs.len()));
                    facts.plan_version = view.version.map(u64::from);
                }
                if let Some(version) = view.version {
                    shown.version(version)?;
                }
                Ok(BoardOutcome::Shown(shown, *view))
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
                        completion_races_lost: own.lost,
                        completion_pause_ms: own.pause_ms,
                        ..completion_facts_of(completion, task.fields().prs.len())
                    });
                }
                Ok(changed(task, completion))
            }
            BoardRequest::Complete(id) => Ok(changed(self.complete(id, facts)?, None)),
            BoardRequest::ProposePlan(proposal) => Ok(BoardOutcome::Planned(
                self.parts.propose_plan.execute_noted(proposal, facts)?,
            )),
            BoardRequest::EditPlan(edit) => Ok(BoardOutcome::Planned(
                self.parts.edit_plan.execute_noted(edit, facts)?,
            )),
            BoardRequest::AcceptPlan(acceptance) => Ok(BoardOutcome::Accepted(Box::new(
                self.parts.accept_plan.execute_noted(acceptance, facts)?,
            ))),
            BoardRequest::Start(start) => Ok(BoardOutcome::Started(Box::new(
                self.parts.start.execute_noted(start, facts)?,
            ))),
            BoardRequest::DiscardPlan(discard) => Ok(BoardOutcome::Planned(
                self.parts.discard_plan.execute_noted(discard, facts)?,
            )),
            BoardRequest::EndRun(end) => Ok(BoardOutcome::RunEnded(
                self.parts.end_run.execute_noted(end, facts)?,
            )),
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

    /// The tasks `list` lists, as `summaries` asks. Asked for none, only
    /// the listing is read. Asked for summaries, each task's is read from
    /// the same board ([`QueryBoard::summarized`]): listed again while the
    /// board moved between the listing and that read, at most
    /// [`MAX_LIST_ATTEMPTS`] times. Past that, or when the listing cannot
    /// be read again, the summaries are skipped as contended; when the
    /// board stands where the listing was read but holds other rows, at
    /// once as the index disagreeing; when the whole read fails, with its
    /// refusal. The listing read last is answered all the same: only the
    /// first listing's own failure is a refusal, as ever.
    ///
    /// [`QueryBoard::summarized`]: super::QueryBoard::summarized
    fn listed(
        &self,
        list: impl Fn() -> Result<BoardListing, BoardStoreError>,
        summaries: ListSummaries,
    ) -> Result<BoardOutcome, ProjectBoardError> {
        let mut listing = list()?;
        match summaries {
            ListSummaries::Omitted => return Ok(BoardOutcome::Listing(Listed::plain(listing))),
            ListSummaries::Wanted => {}
        }
        let mut attempts = 1;
        let skipped = loop {
            match self.parts.query.summarized(&listing) {
                Ok(Summarized::Rows(rows)) => {
                    return Ok(BoardOutcome::Listing(Listed::summarized(listing, rows)));
                }
                Ok(Summarized::Disagrees) => break SkipReason::IndexDisagrees,
                Ok(Summarized::Moved) if attempts < MAX_LIST_ATTEMPTS => {}
                Ok(Summarized::Moved) => break SkipReason::Contended,
                Err(error) => break SkipReason::Failed(Box::new(error)),
            }
            // The board moved since the listing: list it again and read the
            // board whole again. Two whole reads (and more listings) for a
            // board that moved is the cost of an opt-in summary, accepted.
            match list() {
                Ok(again) => listing = again,
                // The board moved, and its listing could not be read again:
                // the listing read before is answered, its board contended.
                Err(_) => break SkipReason::Contended,
            }
            attempts += 1;
        };
        assert!(attempts <= MAX_LIST_ATTEMPTS, "a bounded number of reads");
        Ok(BoardOutcome::Listing(Listed::skipped(listing, skipped)))
    }

    /// Task `id` completed, noted in `facts` when given: with no pull
    /// request host, a task not on the board, or one linking none, is
    /// answered as such first (#2658).
    fn complete(&self, id: &Slug, facts: Option<&mut OpFacts>) -> Result<Task, ProjectBoardError> {
        self.parts.complete.execute_noted(id, facts)
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

#[cfg(test)]
#[path = "run_board_operation_list_tests.rs"]
mod list_tests;
