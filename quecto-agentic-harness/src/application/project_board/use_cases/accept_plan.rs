//! Accepts a draft version of a task's plan (#2481 M4c) as a
//! compare-and-swap: the task, the plan accepted and the plan it
//! supersedes, if one was accepted, are written in one change, task
//! first. The draft must still be as the caller read it
//! (`expected_rev`, #2705 B9), so the content accepted is the content reviewed,
//! and the task must still name the plan the caller saw accepted
//! (`expected_accepted`), so an acceptance never supersedes one its caller
//! did not see and a rollback is a knowing choice;
//! a lost race is judged again on the fresh board, so an edit that landed
//! in between is refused (`plan_changed`), never accepted unread. The
//! claim holder decides for a held task, anyone for one nobody holds, and
//! nothing is accepted while a run of the task is going. Refused while the
//! task's file, or a file of one of its plans or runs, is set aside: what
//! it held (the accepted plan, the open run) is unknown. Blocking: a lost
//! write pauses the calling thread before it is tried again.
use std::sync::Arc;

use super::board_change::{checked_writer, commit, owned, task_on};
use super::board_view;
use super::read_pins::plan_as_read;
use crate::application::project_board::dto::{PlanAcceptance, ProjectBoardError};
use crate::application::project_board::ports::{BoardArtifact, BoardClock, ProjectBoardStore};
use crate::application::project_board::telemetry::OpFacts;
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::plan::Plan;
use crate::domain::project_board::services::plans::{Accepted, AsRead, Moment, TaskPlans, accept};
use crate::domain::project_board::value_objects::vocabulary::PlanState;

pub struct AcceptPlan {
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
    /// The store's committer, checked valid when built: who decides.
    actor: Identity,
}

impl AcceptPlan {
    /// # Errors
    /// The store writes as an invalid identity.
    pub fn new(
        store: Arc<dyn ProjectBoardStore>,
        clock: Arc<dyn BoardClock>,
    ) -> Result<Self, ProjectBoardError> {
        let actor = checked_writer(&*store)?;
        Ok(Self {
            store,
            clock,
            actor,
        })
    }

    /// The acceptance `acceptance` makes, as written.
    ///
    /// # Errors
    /// No such task, or its file, one of its plans' or runs' set aside; the
    /// draft changed since the pinned read; the rules' (a done or archived task, plans or
    /// runs that contradict each other, no such version, not a draft,
    /// changed since it was read, another version accepted than the one
    /// seen, stamped ahead, an actor who may not decide, a run going); the
    /// clock's; a board that kept moving; the store's.
    pub fn execute(&self, acceptance: &PlanAcceptance) -> Result<Accepted, ProjectBoardError> {
        self.execute_noted(acceptance, None)
    }

    /// [`Self::execute`], noting in `facts` (epic #2481 T4), when given,
    /// the version it names, the write's attempts, and the plan's state as
    /// last read and as written.
    ///
    /// # Errors
    /// As [`Self::execute`].
    pub fn execute_noted(
        &self,
        acceptance: &PlanAcceptance,
        mut facts: Option<&mut OpFacts>,
    ) -> Result<Accepted, ProjectBoardError> {
        let noting = facts.is_some();
        if let Some(facts) = facts.as_deref_mut() {
            facts.plan_version = Some(u64::from(acceptance.version));
        }
        let (id, version) = (&acceptance.task, acceptance.version);
        let message = format!("board: accept plan {version} of {}", id.as_str());
        let (store, clock, actor) = (&*self.store, &*self.clock, &self.actor);
        let mut read = None;
        let mut accepted = None;
        let written = commit(
            store,
            clock,
            &message,
            facts.as_deref_mut(),
            |board, when| {
                // This attempt's read, or none: never an earlier one's.
                read = None;
                let task = task_on(board, id)?;
                let versions = owned(board_view::plans_of(board, id)?);
                let runs = owned(board_view::runs_of(board, id)?);
                if noting {
                    read = state_of(&versions, version);
                }
                if let Some(plan) = versions
                    .iter()
                    .find(|plan| plan.fields().version == version)
                {
                    plan_as_read(store, board, plan, Some(&acceptance.expected))?;
                }
                let board = TaskPlans {
                    task,
                    versions: &versions,
                    runs: &runs,
                };
                let expected = Some(AsRead {
                    updated: acceptance.expected.deprecated_updated(),
                    accepted: acceptance.expected_accepted,
                });
                let done = accept(&board, version, expected, actor, &Moment::from(when))?;
                let artifacts = artifacts_of(&done);
                accepted = Some(done);
                Ok(artifacts)
            },
        );
        if let Some(facts) = facts {
            facts.plan_state_from = read;
            facts.plan_state_to = written.as_ref().ok().map(|_| PlanState::Accepted);
        }
        written.map(|artifacts| {
            let accepted = accepted.expect("a written acceptance was built");
            assert_eq!(
                artifacts,
                artifacts_of(&accepted),
                "the acceptance written is the last one built"
            );
            let superseded = accepted
                .superseded
                .as_ref()
                .map(|plan| plan.fields().version);
            assert_eq!(
                superseded, acceptance.expected_accepted,
                "an acceptance supersedes the version its caller saw accepted, or none"
            );
            accepted
        })
    }
}

/// The state of `version` among `versions`, when the task has it.
fn state_of(versions: &[Plan], version: u32) -> Option<PlanState> {
    versions
        .iter()
        .find(|plan| plan.fields().version == version)
        .map(|plan| plan.fields().state)
}

/// What an acceptance writes, in one change: the task first, then the
/// plan accepted and the one it supersedes.
fn artifacts_of(accepted: &Accepted) -> Vec<BoardArtifact> {
    std::iter::once(BoardArtifact::Task(accepted.task.clone()))
        .chain(std::iter::once(BoardArtifact::Plan(accepted.plan.clone())))
        .chain(accepted.superseded.clone().map(BoardArtifact::Plan))
        .collect()
}

#[cfg(test)]
#[path = "accept_plan_tests.rs"]
mod tests;
