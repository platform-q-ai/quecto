//! Discards a draft version of a task's plan (#2481 M4c) as a
//! compare-and-swap; the plan is kept, as discarded. With
//! `expected_updated`, the draft must still be as the caller read it, so
//! a draft edited since is refused (`plan_changed`), never set aside
//! unread. The claim holder decides for a held task, anyone for one
//! nobody holds. Only the plan is written. Blocking: a lost write pauses
//! the calling thread before it is tried again.
use std::sync::Arc;

use super::board_change::{checked_writer, commit, task_on, written_plan};
use super::board_view;
use crate::application::project_board::dto::{PlanDiscard, ProjectBoardError};
use crate::application::project_board::ports::{BoardArtifact, BoardClock, ProjectBoardStore};
use crate::application::project_board::telemetry::OpFacts;
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::plan::Plan;
use crate::domain::project_board::services::plans::{Moment, discard};
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;

pub struct DiscardPlan {
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
    /// The store's committer, checked valid when built: who decides.
    actor: Identity,
}

impl DiscardPlan {
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

    /// The draft `discard` names, discarded, as written.
    ///
    /// # Errors
    /// No such task, or its file or one of its plans' set aside; no plan of
    /// that version; the rules' (a done or archived task, not a draft,
    /// changed since it was read, stamped ahead, an actor who may not
    /// decide); the clock's; a board that kept moving; the store's.
    pub fn execute(&self, discard: &PlanDiscard) -> Result<Plan, ProjectBoardError> {
        self.execute_noted(discard, None)
    }

    /// [`Self::execute`], noting in `facts` (epic #2481 T4), when given,
    /// the version it names, the write's attempts, and the plan's state as
    /// last read and as written.
    ///
    /// # Errors
    /// As [`Self::execute`].
    pub fn execute_noted(
        &self,
        request: &PlanDiscard,
        mut facts: Option<&mut OpFacts>,
    ) -> Result<Plan, ProjectBoardError> {
        let noting = facts.is_some();
        if let Some(facts) = facts.as_deref_mut() {
            facts.plan_version = Some(u64::from(request.version));
        }
        let (id, version) = (&request.task, request.version);
        let message = format!("board: discard plan {version} of {}", id.as_str());
        let (store, clock, actor) = (&*self.store, &*self.clock, &self.actor);
        let mut read = None;
        let written = commit(
            store,
            clock,
            &message,
            facts.as_deref_mut(),
            |board, when| {
                let task = task_on(board, id)?;
                let plan = board_view::plans_of(board, id)?
                    .into_iter()
                    .find(|plan| plan.fields().version == version);
                if noting {
                    // This attempt's read, or none: never an earlier one's.
                    read = plan.map(|plan| plan.fields().state);
                }
                let plan = plan.ok_or(BoardRuleError::UnknownPlanVersion(version))?;
                let expected = request.expected_updated.as_ref();
                let discarded = discard(task, plan, expected, actor, &Moment::from(when))?;
                Ok(vec![BoardArtifact::Plan(discarded)])
            },
        )
        .map(written_plan);
        if let Some(facts) = facts {
            facts.plan_state_from = read;
            facts.plan_state_to = written.as_ref().ok().map(|plan| plan.fields().state);
        }
        written
    }
}

#[cfg(test)]
#[path = "discard_plan_tests.rs"]
mod tests;
