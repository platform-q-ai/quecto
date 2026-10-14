//! Ends a task's open run by hand (S4c's recovery): for a run whose swarm
//! will never fold it back, as when a start's coordinator never launched
//! and its undo could not end the run, or the process died between the
//! start's write and its launch. Without it such a run would hold its task
//! for good.
//!
//! The claim's holder ends it as `succeeded`, `stopped` or `failed`, folded
//! back as a swarm that ended so with nothing reported is ([`run_items`],
//! [`cost_of`]), so a holder can record a success the keeper did not;
//! never while its swarm is still running. As `lost`
//! it is ended by whoever the domain lets ([`end_lost`]: the holder of a
//! live claim, or anyone when nobody claims the task; one who finds the
//! claim expired takes the task over first). With `expected_updated`, the
//! task must still be as the caller read it. Only the run is written: the
//! task is left for its holder to move. Blocking: a lost write pauses the
//! calling thread before it is tried again.
use std::sync::Arc;

use super::board_change::{checked_writer, commit_run, task_on};
use super::board_view;
use crate::application::project_board::dto::{ProjectBoardError, RunEnd, RunEndOutcome};
use crate::application::project_board::fold_back::{
    Evidence, SwarmEnd, SwarmRunView, cost_of, run_items,
};
use crate::application::project_board::ports::{
    BoardArtifact, BoardClock, BoardTextCells, ProjectBoardStore,
};
use crate::application::project_board::telemetry::OpFacts;
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::run::Run;
use crate::domain::project_board::services::claims::claim_facts;
use crate::domain::project_board::services::runs::{
    HeldEnd, Reported, end_held, end_lost, run_and_plan,
};

pub struct EndRun {
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
    cells: Arc<dyn BoardTextCells>,
    /// The store's committer, checked valid when built: who ends the run.
    actor: Identity,
}

impl EndRun {
    /// # Errors
    /// The store writes as an invalid identity.
    pub fn new(
        store: Arc<dyn ProjectBoardStore>,
        clock: Arc<dyn BoardClock>,
        cells: Arc<dyn BoardTextCells>,
    ) -> Result<Self, ProjectBoardError> {
        let actor = checked_writer(&*store)?;
        Ok(Self {
            store,
            clock,
            cells,
            actor,
        })
    }

    /// The run `request` names, ended as it says, as written.
    ///
    /// # Errors
    /// No such task, run or plan version, or one of their files set aside;
    /// a task stamped ahead, or changed since it was read; the rules' (an
    /// actor who may not end it, a run that has ended); the clock's; a
    /// board that kept moving; the store's.
    pub fn execute(&self, request: &RunEnd) -> Result<Run, ProjectBoardError> {
        self.execute_noted(request, None)
    }

    /// [`Self::execute`], noting in `facts` (epic #2481 T4), when given,
    /// the run and the outcome it names, the write's attempts, and the
    /// task's status and claim as last read.
    ///
    /// # Errors
    /// As [`Self::execute`].
    pub fn execute_noted(
        &self,
        request: &RunEnd,
        mut facts: Option<&mut OpFacts>,
    ) -> Result<Run, ProjectBoardError> {
        let noting = facts.is_some();
        if let Some(facts) = facts.as_deref_mut() {
            facts.run = Some(u64::from(request.run));
            facts.run_outcome = Some(request.outcome.outcome());
        }
        let (id, number) = (&request.task, request.run);
        let message = format!(
            "board: run {number} of {} ended, {}",
            id.as_str(),
            request.outcome.as_str()
        );
        let (store, clock, actor) = (&*self.store, &*self.clock, &self.actor);
        let mut read = None;
        let written = commit_run(
            store,
            clock,
            &message,
            facts.as_deref_mut(),
            |board, when| {
                // This attempt's read, or none: never an earlier one's.
                read = None;
                let task = task_on(board, id)?;
                if noting {
                    read = Some((task.fields().status, claim_facts(task, actor, &when.now)));
                }
                when.may_build_on(task)?;
                match &request.expected_updated {
                    Some(expected) if *expected != task.fields().updated => {
                        return Err(ProjectBoardError::Stale {
                            id: id.clone(),
                            expected: expected.clone(),
                            updated: task.fields().updated.clone(),
                        });
                    }
                    Some(_) | None => {}
                }
                let runs = board_view::runs_of(board, id)?;
                let plans = board_view::plans_of(board, id)?;
                let (run, plan) = run_and_plan(&runs, &plans, number)?;
                let held = |end: HeldEnd, swarm: SwarmEnd| {
                    // Nothing reported: every item and the cost as a
                    // fold-back maps a swarm that reported nothing (a
                    // start's undo's included).
                    let view = SwarmRunView {
                        end: swarm,
                        items: vec![],
                        observed_tokens: 0,
                    };
                    let reported = Reported {
                        items: run_items(plan, &view, &*self.cells, Evidence::Left).items,
                        cost: Some(cost_of(&view)),
                    };
                    end_held(task, (run, plan), end, reported, actor, &when.now)
                };
                let ended = match request.outcome {
                    RunEndOutcome::Succeeded => held(HeldEnd::Succeeded, SwarmEnd::Succeeded)?,
                    RunEndOutcome::Stopped => held(HeldEnd::Stopped, SwarmEnd::Cancelled)?,
                    RunEndOutcome::Failed => held(HeldEnd::Failed, SwarmEnd::Failed)?,
                    RunEndOutcome::Lost => end_lost(task, run, actor, &when.now)?,
                };
                Ok(vec![BoardArtifact::Run(ended)])
            },
        );
        if let Some(facts) = facts {
            facts.status_from = read.map(|(status, _)| status);
            facts.claim = read.and_then(|(_, claim)| claim);
        }
        match written?.as_slice() {
            [BoardArtifact::Run(run)] => {
                assert_eq!(
                    run.fields().number,
                    number,
                    "the run ended is the one named"
                );
                Ok(run.clone())
            }
            other => panic!("an end_run writes its run alone, not {other:?}"),
        }
    }
}

#[cfg(test)]
#[path = "end_run_tests.rs"]
mod tests;
