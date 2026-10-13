//! A started task's run folded back once its swarm has ended (S4, #2479):
//! how it ended, where each task item of its plan stood, with owners and
//! evidence, and what it cost, mapped as [`fold_back`](super::super::fold_back)
//! says. The run is written alone, in one commit: the task is left as it
//! is, for its holder (or the playbook) to move.
//!
//! A run folds back once. One that has ended already (folded back, or
//! ended as lost by the merge that completed its task) is
//! [`FoldedBack::Ended`], quietly: nothing is left to fold. A write that
//! lost the race is rebuilt on a fresh board, which also finds a run
//! another writer ended meanwhile.
//!
//! Blocking: the store's calls, and a pause before a retried write, hold
//! the calling thread.
use std::sync::Arc;

use super::board_change::{checked_writer, commit_run, task_on};
use super::board_view;
use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::fold_back::{SwarmRunView, cost_of, run_items};
use crate::application::project_board::ports::{
    BoardArtifact, BoardClock, BoardTextCells, ProjectBoardStore,
};
use crate::application::project_board::telemetry::{FoldBackFacts, OpFacts, counted};
use crate::domain::project_board::entities::run::Run;
use crate::domain::project_board::services::runs::{fold_back, run_and_plan};
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::telemetry::RunOutcomeKind;

/// What came of a fold-back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FoldedBack {
    /// The run as the board now holds it.
    Folded(Run),
    /// The run (by its number) had ended already: nothing was written.
    Ended(u32),
}

pub struct FoldBackRun {
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
    cells: Arc<dyn BoardTextCells>,
}

impl FoldBackRun {
    /// # Errors
    /// The store writes as an invalid identity.
    pub fn new(
        store: Arc<dyn ProjectBoardStore>,
        clock: Arc<dyn BoardClock>,
        cells: Arc<dyn BoardTextCells>,
    ) -> Result<Self, ProjectBoardError> {
        checked_writer(&*store)?;
        Ok(Self {
            store,
            clock,
            cells,
        })
    }

    /// Run `number` of task `id` folded back from `view`, its swarm
    /// ended.
    ///
    /// # Errors
    /// No such task, run or plan version on the board, or one of their
    /// files set aside; the board's rules or the store refusing the run;
    /// the board contended past every attempt; the store's or clock's
    /// failure. Nothing is written.
    pub fn execute(
        &self,
        id: &Slug,
        number: u32,
        view: &SwarmRunView,
    ) -> Result<FoldedBack, ProjectBoardError> {
        self.execute_noted(id, number, view, None)
    }

    /// [`Self::execute`], noting in `facts` (epic #2481, S4), when given,
    /// the write's attempts and, for a run it folds, how it ended, its
    /// task items by state, what was left out, its cost and its size.
    ///
    /// # Errors
    /// As [`Self::execute`].
    pub fn execute_noted(
        &self,
        id: &Slug,
        number: u32,
        view: &SwarmRunView,
        facts: Option<&mut FoldBackFacts>,
    ) -> Result<FoldedBack, ProjectBoardError> {
        let outcome = view.end.outcome();
        let message = format!(
            "board: run {number} of {} folded back, {}",
            id.as_str(),
            outcome.as_str()
        );
        let cost = cost_of(view);
        let mut write = facts.as_ref().map(|_| OpFacts::default());
        let mut noted = None;
        let (store, clock) = (&*self.store, &*self.clock);
        let written = commit_run(store, clock, &message, write.as_mut(), |board, when| {
            noted = None;
            task_on(board, id)?;
            let runs = board_view::runs_of(board, id)?;
            let plans = board_view::plans_of(board, id)?;
            let (run, plan) = run_and_plan(&runs, &plans, number)?;
            let items = run_items(plan, view, &*self.cells);
            let tally = (
                items.counts(),
                items.unmatched,
                items.extra,
                items.evidence,
                items.dropped,
            );
            let folded = fold_back(run, plan, outcome, Some(cost), items.items, &when.now)?;
            noted = Some(tally);
            Ok(vec![BoardArtifact::Run(folded)])
        });
        let folded = match written {
            Ok(artifacts) => FoldedBack::Folded(written_run(artifacts)),
            Err(ProjectBoardError::Rule(BoardRuleError::RunEnded(ended))) => {
                assert_eq!(ended, number, "the run that ended is the one folded");
                noted = None;
                FoldedBack::Ended(ended)
            }
            Err(error) => return Err(error),
        };
        if let Some(facts) = facts {
            let write = write.unwrap_or_default();
            facts.attempts = write.attempts;
            facts.lost = write.lost;
            if let (FoldedBack::Folded(run), Some((items, unmatched, extra, kept, dropped))) =
                (&folded, noted)
            {
                facts.run_outcome = Some(RunOutcomeKind::of(outcome));
                facts.items = Some(items);
                facts.unmatched = Some(counted(unmatched));
                facts.extra = Some(counted(extra));
                facts.evidence = Some(counted(kept));
                facts.evidence_dropped = Some(counted(dropped));
                facts.cost_tokens = run.fields().cost;
                facts.bytes = Some(counted(run.fields().text_bytes()));
            }
        }
        Ok(folded)
    }
}

/// The run a fold-back wrote: [`commit_run`] holds it to one run alone.
fn written_run(artifacts: Vec<BoardArtifact>) -> Run {
    match artifacts.as_slice() {
        [BoardArtifact::Run(run)] => run.clone(),
        other => panic!("a fold-back writes its run alone, not {other:?}"),
    }
}

#[cfg(test)]
#[path = "fold_back_run_tests.rs"]
mod tests;
