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
use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::{
    BoardArtifact, BoardClock, ProjectBoardStore, PullRequestState, PullRequestStates,
    PullRequestStatesError,
};
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::services::transitions::{
    Completed, completable, complete_merged,
};
use crate::domain::project_board::value_objects::slug::Slug;

/// The time a completion's pull request reads share.
pub const READ_BUDGET: Duration = Duration::from_secs(30);

pub struct CompleteMergedTask {
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
    prs: Arc<dyn PullRequestStates>,
    read_budget: Duration,
}

impl CompleteMergedTask {
    /// # Errors
    /// The store writes as an invalid identity.
    pub fn new(
        store: Arc<dyn ProjectBoardStore>,
        clock: Arc<dyn BoardClock>,
        prs: Arc<dyn PullRequestStates>,
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
        let message = format!("board: {} done, its pull requests merged", id.as_str());
        // One deadline for every attempt: a lost write does not restart it.
        let deadline = self.clock.elapsed().saturating_add(self.read_budget);
        let written = commit(&*self.store, &*self.clock, &message, |board, when| {
            let task = task_on(board, id)?;
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
            let reads = Reads {
                host: &*self.prs,
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
        })?;
        Ok(written_task(written))
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
