//! A started task's run folded back once its swarm has ended (S4, #2479):
//! how it ended, where each task item of its plan stood, with owners and
//! evidence, and what it cost, mapped as [`fold_back`](super::super::fold_back)
//! says. The run's end moves its task as its swarm ended (#2705 T5:
//! `run_outcomes::task_after_end`), task and run in one commit.
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

use super::board_change::{checked_writer, commit_run, run_end_writes, task_on, written_run_end};
use super::board_view;
use crate::application::project_board::dto::{FoldBackFacts, FoldedBack, ProjectBoardError};
use crate::application::project_board::fold_back::{
    Evidence, FoldBackItems, SwarmRunView, cost_of, run_items,
};
use crate::application::project_board::ports::{
    BoardClock, BoardStoreError, BoardTextCells, FoldBackOutcome, FoldBackRecorder,
    ProjectBoardStore,
};
use crate::application::project_board::telemetry::{OpFacts, counted, summed};
use crate::domain::project_board::entities::run::Run;
use crate::domain::project_board::services::run_outcomes::{task_after_end, task_after_undo};
use crate::domain::project_board::services::runs::{fold_back, run_and_plan};
use crate::domain::project_board::value_objects::code_ref::CommitId;
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::telemetry::RunOutcomeKind;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;

pub struct FoldBackRun {
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
    cells: Arc<dyn BoardTextCells>,
}

/// Whose end a fold-back records: a swarm's, or a start's undo, which
/// names the status the start took its task from.
#[derive(Debug, Clone, Copy)]
enum Ending {
    Swarm,
    Undo(TaskStatus),
}

/// One pass at the write: the run written and what its items kept.
type Pass = Result<(Run, FoldBackItems), ProjectBoardError>;

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
    /// ended. A run that would leave the board past its bytes
    /// ([`BoardStoreError::WriteTooLarge`]) is folded once more without its
    /// evidence: its outcome, states, owners and cost. Any other invalid
    /// write is refused as it is, its evidence never left out.
    ///
    /// # Errors
    /// No such task, run or plan version on the board, or one of their
    /// files set aside; the board's rules or the store refusing the run
    /// (as invalid, or too large even without its evidence);
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

    /// [`Self::execute`], its outcome handed to `recorder` when there is
    /// one, with what it saw and how long it took; with none, nothing is
    /// gathered or timed.
    ///
    /// # Errors
    /// As [`Self::execute`].
    pub fn execute_recorded(
        &self,
        id: &Slug,
        number: u32,
        view: &SwarmRunView,
        recorder: Option<&dyn FoldBackRecorder>,
    ) -> Result<FoldedBack, ProjectBoardError> {
        let Some(recorder) = recorder else {
            return self.execute(id, number, view);
        };
        let started = self.clock.elapsed();
        let mut facts = FoldBackFacts::default();
        let result = self.execute_noted(id, number, view, Some(&mut facts));
        recorder.record(FoldBackOutcome {
            task: id,
            run: number,
            result: &result,
            facts: &facts,
            elapsed: self.clock.elapsed().saturating_sub(started),
        });
        result
    }

    /// [`Self::execute`], noting in `facts` (epic #2481, S4), when given,
    /// its writes' attempts and lost races, whatever came of them, and,
    /// for a run it folds, how it ended, its task items by state, what was
    /// left out, its cost and its size.
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
        self.folded(id, number, (view, Ending::Swarm), facts)
    }

    /// [`Self::execute_noted`] for the undo of a start whose coordinator
    /// never launched (#2705 T5 review round 1): a task the start took
    /// from review or blocked (`from`) goes back there in the same write,
    /// its claim kept (`run_outcomes::task_after_undo`).
    ///
    /// # Errors
    /// As [`Self::execute`].
    pub fn undo_noted(
        &self,
        id: &Slug,
        number: u32,
        (view, from): (&SwarmRunView, TaskStatus),
        facts: Option<&mut FoldBackFacts>,
    ) -> Result<FoldedBack, ProjectBoardError> {
        self.folded(id, number, (view, Ending::Undo(from)), facts)
    }

    fn folded(
        &self,
        id: &Slug,
        number: u32,
        (view, ending): (&SwarmRunView, Ending),
        facts: Option<&mut FoldBackFacts>,
    ) -> Result<FoldedBack, ProjectBoardError> {
        let mut write = facts.as_ref().map(|_| OpFacts::default());
        let mut passed = self.pass(id, number, (view, ending), Evidence::Kept, write.as_mut());
        // Only a run past the board's bytes is folded again, smaller (an
        // allowlist): any other invalid write is the caller's to see.
        if let Err(ProjectBoardError::Store(BoardStoreError::WriteTooLarge(_))) = &passed {
            let mut again = facts.as_ref().map(|_| OpFacts::default());
            passed = self.pass(id, number, (view, ending), Evidence::Left, again.as_mut());
            if let (Some(write), Some(again)) = (write.as_mut(), again) {
                write.attempts = summed(write.attempts, again.attempts);
                write.lost = summed(write.lost, again.lost);
                write.lock_races = summed(write.lock_races, again.lock_races);
                assert!(write.attempts.is_some(), "a noted write notes its attempts");
                write.head = again.head;
            }
        }
        let (result, items) = match passed {
            Ok((run, items)) => (Ok(FoldedBack::Folded(run)), Some(items)),
            Err(ProjectBoardError::Rule(BoardRuleError::RunEnded(ended))) => {
                assert_eq!(ended, number, "the run that ended is the one folded");
                (Ok(FoldedBack::Ended(ended)), None)
            }
            Err(error) => (Err(error), None),
        };
        noted(facts, write, result, items)
    }

    /// One pass at folding the run back, its evidence kept or left out.
    fn pass(
        &self,
        id: &Slug,
        number: u32,
        (view, ending): (&SwarmRunView, Ending),
        evidence: Evidence,
        write: Option<&mut OpFacts>,
    ) -> Pass {
        let outcome = view.end.outcome();
        let message = format!(
            "board: run {number} of {} folded back, {}",
            id.as_str(),
            outcome.as_str()
        );
        let mut kept = None;
        let (store, clock) = (&*self.store, &*self.clock);
        let written = commit_run(store, clock, &message, write, |board, when| {
            let task = task_on(board, id)?;
            let runs = board_view::runs_of(board, id)?;
            let plans = board_view::plans_of(board, id)?;
            let (run, plan) = run_and_plan(&runs, &plans, number)?;
            let items = run_items(plan, view, &*self.cells, evidence);
            let cost = Some(cost_of(view));
            let folded = fold_back(run, plan, outcome, cost, items.items.clone(), &when.now)?;
            // The run's end moves its task, in the same write (#2705 T5).
            let moved = match ending {
                Ending::Swarm => task_after_end(task, &folded, view.end.stop(), &when.now)?,
                Ending::Undo(from) => task_after_undo(task, &folded, from, &when.now)?,
            };
            // This attempt's items, never an earlier one's.
            kept = Some(items);
            Ok(run_end_writes(folded, moved))
        })?;
        let items = kept.expect("a written fold-back mapped its items");
        let (_, run) = written_run_end(&written);
        Ok((run.clone(), items))
    }
}

/// `result`, after noting in `facts`, when given, the writes'
/// attempts and, for a run folded, what its `items` kept.
fn noted(
    facts: Option<&mut FoldBackFacts>,
    write: Option<OpFacts>,
    result: Result<FoldedBack, ProjectBoardError>,
    items: Option<FoldBackItems>,
) -> Result<FoldedBack, ProjectBoardError> {
    let Some(facts) = facts else {
        return result;
    };
    let write = write.unwrap_or_default();
    facts.attempts = write.attempts;
    facts.lost = write.lost;
    facts.lock_races = write.lock_races;
    facts.head = write.head.and_then(|head| CommitId::parse(&head.0).ok());
    if let (Ok(FoldedBack::Folded(run)), Some(items)) = (&result, items) {
        let fields = run.fields();
        facts.run_outcome = fields.outcome.map(RunOutcomeKind::of);
        facts.items = Some(items.counts());
        facts.unmatched = Some(counted(items.unmatched));
        facts.extra = Some(counted(items.extra));
        facts.evidence = Some(counted(items.evidence));
        facts.evidence_dropped = Some(counted(items.dropped));
        facts.cost_tokens = fields.cost;
        facts.bytes = Some(counted(fields.text_bytes()));
    }
    result
}

#[cfg(test)]
#[path = "fold_back_run_tests.rs"]
mod tests;
