//! The system's move behind "a merged PR moves its task to done" (epic
//! #2481): a task (ready or held) whose linked pull requests have all
//! merged is done, whoever holds it; its claim is cleared and an open run
//! ended as lost. Not an agent's operation: S2 wires it from the
//! repository's merge events only. Each event just asks again: the
//! states of every linked pull request are read from the host
//! ([`PullRequestStates`]) and the task completes only when all read
//! merged, so a task linking several closes on the last merge. Blocking:
//! the host's reads, and a pause before a retried write, hold the calling
//! thread.
use std::collections::BTreeSet;
use std::sync::Arc;

use super::board_change::{checked_writer, commit, task_on};
use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::{
    BoardClock, ProjectBoardStore, PullRequestState, PullRequestStates,
};
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::services::transitions::complete_merged;
use crate::domain::project_board::value_objects::slug::Slug;

pub struct CompleteMergedTask {
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
    prs: Arc<dyn PullRequestStates>,
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
        Ok(Self { store, clock, prs })
    }

    /// Task `id` done, when every pull request it links reads merged.
    ///
    /// # Errors
    /// A pull request the host does not know, or the host's failure; a
    /// pull request not merged, and the rest as
    /// [`ChangeBoardTask`](super::ChangeBoardTask)'s. Nothing is written.
    pub fn execute(&self, id: &Slug) -> Result<Task, ProjectBoardError> {
        let message = format!("board: {} done, its pull requests merged", id.as_str());
        let tasks = commit(&*self.store, &*self.clock, &message, |board, when| {
            let task = task_on(board, id)?;
            when.may_build_on(task)?;
            let merged = merged(&*self.prs, &task.fields().prs)?;
            Ok(vec![
                complete_merged(task, &merged, &when.now).map_err(ProjectBoardError::Rule)?,
            ])
        })?;
        Ok(tasks.into_iter().next().expect("a change writes its task"))
    }
}

/// The linked pull requests the host reads as merged; an open or closed
/// one is left out, for the rule to name. One the host does not know, or
/// a host that cannot be read, refuses the completion.
fn merged(
    host: &dyn PullRequestStates,
    linked: &[u64],
) -> Result<BTreeSet<u64>, ProjectBoardError> {
    let mut merged = BTreeSet::new();
    for pr in linked {
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
