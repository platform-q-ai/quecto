//! Proposes a new draft version of a task's plan (#2481 M4b) as a
//! compare-and-swap, as every change is: its number follows the task's
//! last version as last read, so two proposals that race get two
//! versions. Anyone with access to the board proposes, as the store's
//! committer; a done or archived task's plans are settled. Its text is
//! normalised as it enters ([`plan_text`](super::plan_text)). A team
//! whose coordinator role's model names no provider is refused, as a
//! start would refuse it. Only the plan is written: the task is left as
//! it is. Refused while any plan of
//! the task is set aside, since the next number may be that file's.
//! Blocking: a lost write pauses the calling thread before it is tried
//! again.
use std::sync::Arc;

use super::board_change::{checked_writer, commit, owned, task_on, written_plan};
use super::{board_view, plan_text};
use crate::application::project_board::dto::{PlanProposal, ProjectBoardError};
use crate::application::project_board::ports::{BoardArtifact, BoardClock, ProjectBoardStore};
use crate::application::project_board::swarm_brief::coordinator_model_named;
use crate::application::project_board::telemetry::OpFacts;
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::plan::Plan;
use crate::domain::project_board::services::plans::{Moment, PlanContent, propose_at};
use crate::domain::project_board::value_objects::board_text;

pub struct ProposePlan {
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
    /// The store's committer, checked valid when built: who proposes.
    actor: Identity,
}

impl ProposePlan {
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

    /// The new draft `proposal` makes, as written.
    ///
    /// # Errors
    /// No such task, or its file or one of its plans' set aside; a done or
    /// archived task, plans that contradict each other, no version left,
    /// or the plan's schema; a coordinator model that names no provider;
    /// the clock's; a board that kept moving; the
    /// store's.
    pub fn execute(&self, proposal: &PlanProposal) -> Result<Plan, ProjectBoardError> {
        self.execute_noted(proposal, None)
    }

    /// [`Self::execute`], noting in `facts` (epic #2481 T4), when given,
    /// the write's attempts, what the normaliser changed in the approach
    /// and the task items, and the version written with its task items
    /// and team roles.
    ///
    /// # Errors
    /// As [`Self::execute`].
    pub fn execute_noted(
        &self,
        proposal: &PlanProposal,
        mut facts: Option<&mut OpFacts>,
    ) -> Result<Plan, ProjectBoardError> {
        if let Some(facts) = facts.as_deref_mut() {
            let (approach, items) = (proposal.approach.as_str(), proposal.items.as_slice());
            facts.normalised = Some(plan_text::normalised(Some(approach), Some(items)));
        }
        let content = PlanContent {
            approach: board_text::text(&proposal.approach),
            handoff: proposal.handoff.as_deref().and_then(plan_text::handoff),
            items: plan_text::entered_items(&proposal.items),
            team: proposal.team.clone(),
            entry: proposal.entry,
        };
        let id = &proposal.task;
        let message = format!("board: propose a plan for {}", id.as_str());
        let (store, clock, actor) = (&*self.store, &*self.clock, &self.actor);
        let written = commit(
            store,
            clock,
            &message,
            facts.as_deref_mut(),
            |board, when| {
                let task = task_on(board, id)?;
                let versions = owned(board_view::plans_of(board, id)?);
                let plan =
                    propose_at(task, &versions, content.clone(), actor, &Moment::from(when))?;
                coordinator_model_named(plan.fields().team.as_ref())?;
                Ok(vec![BoardArtifact::Plan(plan)])
            },
        )
        .map(written_plan);
        if let (Some(facts), Ok(plan)) = (facts, &written) {
            plan_text::note_written(facts, plan);
        }
        written
    }
}

#[cfg(test)]
#[path = "propose_plan_tests.rs"]
mod tests;
