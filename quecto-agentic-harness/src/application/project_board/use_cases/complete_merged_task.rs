//! The system's move behind "a merged PR moves its task to done" (epic
//! #2481): a task (ready or held) whose linked pull requests have all
//! merged is done, whoever holds it; its claim is cleared and an open run
//! ended as lost. The states of every linked pull request are read from
//! the host ([`PullRequestStates`]), never taken from the caller, so it is
//! safe for anyone to run, and running it again only asks again: a task
//! linking several closes on the run after the last merge.
//!
//! A change of the task still open (linked, its merge not recorded) holds
//! the completion: refused as `changes_unmerged` before the host is read.
//! A task is completed through its changes by M4's `changes::complete`,
//! wired with M4's link op (it needs the host's merge commits, which
//! [`PullRequestStates`] does not read), so no change exists before one
//! can complete it.
//!
//! S2 runs it after every successful `AddPr` (a pull request may have
//! merged before it was linked, and no merge event follows), on every
//! `Move(Review)`, and on the repository's merge events; and the holder or
//! any user can run it as a "complete" op.
//!
//! Blocking: the host's reads, and a pause before a retried write, hold
//! the calling thread. All of a completion's reads, over every attempt at
//! its write, share one time budget ([`READ_BUDGET`]), measured on the
//! board clock's monotonic time: once it is spent no further pull request
//! is asked about, so a completion reads the host for at most the budget
//! plus one read's own timeout.
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use super::board_change::{checked_writer, commit, owned, task_on, written_task};
use super::board_view;
use crate::application::project_board::dto::{NoPullRequestHost, ProjectBoardError};
use crate::application::project_board::ports::{
    BoardArtifact, BoardClock, ProjectBoardStore, PullRequestState, PullRequestStates,
    PullRequestStatesError,
};
use crate::application::project_board::telemetry::{OpFacts, completion_facts};
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::services::transitions::{
    Completed, completable, complete_merged,
};
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::telemetry::CompletionKind;

/// The time a completion's pull request reads share.
pub const READ_BUDGET: Duration = Duration::from_secs(30);

pub struct CompleteMergedTask {
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
    /// The host pull request states are read from, or why there is none:
    /// a completion asks it only about a task that links pull requests
    /// (#2658).
    prs: Result<Arc<dyn PullRequestStates>, NoPullRequestHost>,
    read_budget: Duration,
}

impl CompleteMergedTask {
    /// A completion reading `prs`, or with none, `prs`'s reason: then a
    /// task that links pull requests is refused as `no_pull_request_host`,
    /// and any other is judged by the rule alone (#2658).
    ///
    /// # Errors
    /// The store writes as an invalid identity.
    pub fn reading(
        store: Arc<dyn ProjectBoardStore>,
        clock: Arc<dyn BoardClock>,
        prs: Result<Arc<dyn PullRequestStates>, NoPullRequestHost>,
    ) -> Result<Self, ProjectBoardError> {
        checked_writer(&*store)?;
        Ok(Self {
            store,
            clock,
            prs,
            read_budget: READ_BUDGET,
        })
    }

    /// The same, its reads sharing `budget` instead of [`READ_BUDGET`].
    pub fn with_read_budget(mut self, budget: Duration) -> Self {
        self.read_budget = budget;
        self
    }

    /// Task `id` done, when every pull request it links reads merged and
    /// no change of it is open; its run going, if one is, is ended as lost
    /// in the same write.
    ///
    /// # Errors
    /// A pull request the host does not know, or the host's failure; a
    /// pull request not merged, and the rest as
    /// [`ChangeBoardTask`](super::ChangeBoardTask)'s. Nothing is written.
    pub fn execute(&self, id: &Slug) -> Result<Task, ProjectBoardError> {
        self.execute_noted(id, None)
    }

    /// [`Self::execute`], noting in `facts` (epic #2481 T4), when given,
    /// the write's attempts, the task's status as last read and as
    /// written, and what came of completing it: done, or pending on pull
    /// requests not merged yet (any other refusal is its own answer).
    ///
    /// # Errors
    /// As [`Self::execute`].
    pub fn execute_noted(
        &self,
        id: &Slug,
        mut facts: Option<&mut OpFacts>,
    ) -> Result<Task, ProjectBoardError> {
        let message = format!("board: {} done, its pull requests merged", id.as_str());
        // One deadline for every attempt: a lost write does not restart it.
        let deadline = self.clock.elapsed().saturating_add(self.read_budget);
        let noting = facts.is_some();
        let mut read = None;
        let noted = facts.as_deref_mut();
        let (store, clock) = (&*self.store, &*self.clock);
        let written = commit(store, clock, &message, noted, |board, when| {
            let task = task_on(board, id);
            if noting {
                // This attempt's read, or none: never an earlier one's.
                read = task
                    .as_ref()
                    .ok()
                    .map(|task| (task.fields().status, task.fields().prs.len()));
            }
            let task = task?;
            when.may_build_on(task)?;
            // A task the rule would not complete is refused as such, before
            // its changes and runs are read.
            completable(task)?;
            // The task's changes and runs, as the board holds them: an open
            // change holds the completion, and the run going is written back
            // ended. What a set-aside one held is unknown, so the
            // completion waits for it.
            let changes = owned(board_view::changes_of(board, id)?);
            let runs = owned(board_view::runs_of(board, id)?);
            // The host is asked only about a task that would complete were
            // its pull requests merged: one linking none, or one an open
            // change holds, is refused by the rule first.
            let linked: BTreeSet<u64> = task.fields().prs.iter().copied().collect();
            complete_merged(task, &linked, &changes, &runs, &when.now)?;
            // Only now is there a pull request to ask a host about.
            let host = self
                .prs
                .as_ref()
                .map_err(|why| ProjectBoardError::NoPullRequestHost(why.clone()))?;
            assert!(
                !linked.is_empty(),
                "the rule asks the host only about links"
            );
            let reads = Reads {
                host: &**host,
                clock: &*self.clock,
                deadline,
                budget: self.read_budget,
            };
            let merged = merged(&reads, &task.fields().prs)?;
            let Completed { task, run } =
                complete_merged(task, &merged, &changes, &runs, &when.now)?;
            // The task and the run it ended, in one change.
            let mut written = vec![BoardArtifact::Task(task)];
            written.extend(run.map(BoardArtifact::Run));
            Ok(written)
        });
        let written = written.map(written_task);
        if let (Some(facts), Some((status, linked))) = (facts, read) {
            facts.status_from = Some(status);
            facts.status_to = written.as_ref().ok().map(|task| task.fields().status);
            facts.completion = match &written {
                Ok(task) => Some(completion_facts(
                    CompletionKind::Done,
                    task.fields().prs.len(),
                    None,
                )),
                Err(ProjectBoardError::Rule(BoardRuleError::PrsUnmerged(prs))) => Some(
                    completion_facts(CompletionKind::Pending, linked, Some(prs.len())),
                ),
                Err(_) => None,
            };
        }
        written
    }
}

/// The host a completion reads, until `deadline` on `clock`'s monotonic
/// time (`budget` after the completion began).
struct Reads<'a> {
    host: &'a dyn PullRequestStates,
    clock: &'a dyn BoardClock,
    deadline: Duration,
    budget: Duration,
}

/// The linked pull requests the host reads as merged; an open or closed
/// one is left out, for the rule to name. One the host does not know, a
/// host that cannot be read, or reads past the deadline refuse the
/// completion.
fn merged(reads: &Reads<'_>, linked: &[u64]) -> Result<BTreeSet<u64>, ProjectBoardError> {
    let host = reads.host;
    let mut merged = BTreeSet::new();
    for pr in linked {
        if reads.clock.elapsed() >= reads.deadline {
            return Err(ProjectBoardError::PullRequests(
                PullRequestStatesError::Unavailable(format!(
                    "reading {} pull requests took longer than {} s",
                    linked.len(),
                    reads.budget.as_secs()
                )),
            ));
        }
        match host.state(*pr).map_err(ProjectBoardError::PullRequests)? {
            PullRequestState::Merged => {
                merged.insert(*pr);
            }
            PullRequestState::Open | PullRequestState::Closed => {}
            PullRequestState::Unknown => return Err(ProjectBoardError::PrStateUnknown(*pr)),
        }
    }
    Ok(merged)
}

#[cfg(test)]
#[path = "complete_merged_task_tests.rs"]
mod tests;
