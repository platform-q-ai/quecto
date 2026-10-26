//! A task done without a run (#2705 T4): work finished with no run of its
//! plan, saying why, by the board's rule ([`done_without_run`]). A task
//! that links pull requests is done so only once every one has merged (the
//! owner's choice, T4 review round 1): their states are read from the host
//! as a completion reads them ([`CompleteMergedTask`]'s budgeted reads,
//! [`READ_BUDGET`] over every attempt), never taken from the caller. The
//! host is asked only about a task the rule would finish were its pull
//! requests merged, so a refusal for anything else reads nothing there.
//!
//! The merged-pull-request completion itself stays review-only
//! (`complete`). A board server pre-reads a completion's pull request
//! states before its write lock; this op needs the same once the server
//! takes writes (#2705 B7/B8).
//!
//! Blocking: the host's reads, and a pause before a retried write, hold
//! the calling thread.
//!
//! [`CompleteMergedTask`]: super::CompleteMergedTask
use std::collections::BTreeSet;
use std::sync::Arc;

use super::board_change::{checked_writer, commit, owned, task_on, written_task};
use super::board_view;
use super::complete_merged_task::{READ_BUDGET, Reads, merged};
use crate::application::project_board::dto::{NoPullRequestHost, ProjectBoardError};
use crate::application::project_board::ports::{
    BoardArtifact, BoardClock, ProjectBoardStore, PullRequestStates,
};
use crate::application::project_board::telemetry::OpFacts;
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::services::claims::claim_facts;
use crate::domain::project_board::services::transitions::{
    DoneWork, done_without_run, done_without_run_reads,
};
use crate::domain::project_board::value_objects::board_text;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;

pub struct FinishWithoutRun {
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
    /// The host linked pull requests are read from, or why there is none.
    prs: Result<Arc<dyn PullRequestStates>, NoPullRequestHost>,
    /// The store's committer, checked valid when built.
    actor: Identity,
}

impl FinishWithoutRun {
    /// # Errors
    /// The store writes as an invalid identity.
    pub fn reading(
        store: Arc<dyn ProjectBoardStore>,
        clock: Arc<dyn BoardClock>,
        prs: Result<Arc<dyn PullRequestStates>, NoPullRequestHost>,
    ) -> Result<Self, ProjectBoardError> {
        let actor = checked_writer(&*store)?;
        Ok(Self {
            store,
            clock,
            prs,
            actor,
        })
    }

    /// Task `id` done without a run for `reason` (normalised as one line),
    /// made by the store's committer, noting in `facts`, when given, the
    /// write's attempts and the task's status as last read and as asked
    /// (`done`), and the claim it found.
    ///
    /// # Errors
    /// The board's rules (against the latest task, its runs and changes,
    /// and every task the board holds); a linked pull request not merged
    /// (`prs_open`), one the host does not know, a host that cannot be
    /// read in time, or none to read; the clock's; the store's.
    pub fn execute_noted(
        &self,
        id: &Slug,
        reason: &str,
        mut facts: Option<&mut OpFacts>,
    ) -> Result<Task, ProjectBoardError> {
        let message = format!("board: done {} without a run", id.as_str());
        let reason = board_text::line(reason);
        // One deadline for every attempt: a lost write does not restart it.
        let budget = READ_BUDGET;
        let deadline = self.clock.elapsed().saturating_add(budget);
        let noting = facts.is_some();
        let mut read = None;
        let actor = &self.actor;
        let (store, clock) = (&*self.store, &*self.clock);
        let written = commit(
            store,
            clock,
            &message,
            facts.as_deref_mut(),
            |board, when| {
                let task = task_on(board, id);
                if noting {
                    // This attempt's read, or none: never an earlier one's.
                    let found = task.as_ref().ok();
                    read = found
                        .map(|task| (task.fields().status, claim_facts(task, actor, &when.now)));
                }
                let task = task?;
                let reads = done_without_run_reads(task, actor, &when.now)?;
                let runs = match reads.runs {
                    true => owned(board_view::runs_of(board, id)?),
                    false => vec![],
                };
                let changes = match reads.changes {
                    true => owned(board_view::changes_of(board, id)?),
                    false => vec![],
                };
                // A set-aside task file may be a child or dependency it waits for.
                board_view::tasks_clear(board)?;
                let linked: BTreeSet<u64> = task.fields().prs.iter().copied().collect();
                let work = |merged| DoneWork {
                    runs: &runs,
                    changes: &changes,
                    tasks: &board.tasks,
                    merged,
                };
                // The rule first, as if every linked pull request had merged:
                // only a task it would finish has its pull requests read.
                let done = done_without_run(task, &reason, actor, work(&linked), &when.now)?;
                let done = match linked.is_empty() {
                    true => done,
                    false => {
                        let host = self
                            .prs
                            .as_ref()
                            .map_err(|why| ProjectBoardError::NoPullRequestHost(why.clone()))?;
                        let reads = Reads {
                            host: &**host,
                            clock,
                            deadline,
                            budget,
                        };
                        let merged = merged(&reads, &task.fields().prs)?;
                        done_without_run(task, &reason, actor, work(&merged), &when.now)?
                    }
                };
                Ok(vec![BoardArtifact::Task(done)])
            },
        );
        let written = written.map(written_task);
        if let Some(facts) = facts {
            if let Some((status, claim)) = read {
                facts.status_from = Some(status);
                facts.claim = claim;
            }
            facts.status_to = Some(TaskStatus::Done);
        }
        written
    }
}

#[cfg(test)]
#[path = "finish_without_run_tests.rs"]
mod tests;
