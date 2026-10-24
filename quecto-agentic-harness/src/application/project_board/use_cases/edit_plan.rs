//! Edits a draft version of a task's plan (#2481 M4b) as a patch and a
//! compare-and-swap: each attempt reads the draft afresh and replaces only
//! the fields the edit gives, so a field left out keeps what the board
//! holds, byte for byte, even after a lost race, and an agent that saw
//! only part of a long approach never writes back that part. A caller
//! that read the draft passes the `updated` it read, and a draft changed
//! since is refused (`plan_changed`), never edited over. Drafts are
//! anyone's, edited as the store's committer; a done or archived task's
//! plans are settled. Given text is normalised as it enters
//! ([`plan_text`](super::plan_text)); only the plan is written. A draft
//! whose team, given or kept, has a coordinator role whose model names
//! no provider is refused, as a start would refuse it. Blocking:
//! a lost write pauses the calling thread before it is tried again.
use std::sync::Arc;

use super::board_change::{checked_writer, commit, owned, task_on, written_plan};
use super::{board_view, plan_text};
use crate::application::project_board::dto::{PlanEdit, ProjectBoardError};
use crate::application::project_board::ports::{BoardArtifact, BoardClock, ProjectBoardStore};
use crate::application::project_board::swarm_brief::coordinator_model_named;
use crate::application::project_board::telemetry::OpFacts;
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::plan::{Plan, PlanEntry, PlanFields, PlanItem};
use crate::domain::project_board::entities::team::Team;
use crate::domain::project_board::services::plans::{DraftOf, Moment, PlanContent, edit_draft};
use crate::domain::project_board::value_objects::board_text;
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;

pub struct EditPlan {
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
    /// The store's committer, checked valid when built: who edits.
    actor: Identity,
}

impl EditPlan {
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

    /// The draft `edit` names, with the fields it gives replaced, as
    /// written.
    ///
    /// # Errors
    /// No such task, or its file or one of its plans' set aside; no plan
    /// of that version; a done or archived task, a plan that is not a
    /// draft, changed since it was read or stamped ahead, an edit that
    /// changes nothing, or the plan's schema; a coordinator model that
    /// names no provider; the clock's; a board that
    /// kept moving; the store's.
    pub fn execute(&self, edit: &PlanEdit) -> Result<Plan, ProjectBoardError> {
        self.execute_noted(edit, None)
    }

    /// [`Self::execute`], noting in `facts` (epic #2481 T4), when given,
    /// the version the edit names, the write's attempts, what the
    /// normaliser changed in the approach and task items it gives, and the
    /// task items and team roles of the plan written.
    ///
    /// # Errors
    /// As [`Self::execute`].
    pub fn execute_noted(
        &self,
        edit: &PlanEdit,
        mut facts: Option<&mut OpFacts>,
    ) -> Result<Plan, ProjectBoardError> {
        if let Some(facts) = facts.as_deref_mut() {
            facts.plan_version = Some(u64::from(edit.version));
            let (approach, items) = (edit.approach.as_deref(), edit.items.as_deref());
            // An edit that gives no text has nothing normalised to tell.
            facts.normalised = (approach.is_some() || items.is_some())
                .then(|| plan_text::normalised(approach, items));
        }
        let given = PlanPatch {
            approach: edit.approach.as_deref().map(board_text::text),
            handoff: edit.handoff.as_deref().map(plan_text::handoff),
            items: edit.items.as_deref().map(plan_text::entered_items),
            team: edit.team.clone(),
            entry: edit.entry,
        };
        let id = &edit.task;
        let message = format!("board: edit plan {} of {}", edit.version, id.as_str());
        let (store, clock, actor) = (&*self.store, &*self.clock, &self.actor);
        let written = commit(
            store,
            clock,
            &message,
            facts.as_deref_mut(),
            |board, when| {
                let task = task_on(board, id)?;
                let versions = owned(board_view::plans_of(board, id)?);
                let plan = versions
                    .iter()
                    .find(|plan| plan.fields().version == edit.version)
                    .ok_or(BoardRuleError::UnknownPlanVersion(edit.version))?;
                let expected = edit.expected_updated.as_ref();
                let content = given.applied_to(plan.fields());
                let edited = edit_draft(
                    DraftOf {
                        task,
                        versions: &versions,
                        plan,
                    },
                    content,
                    expected,
                    actor,
                    &Moment::from(when),
                )?;
                given.kept_in(plan.fields(), edited.fields());
                // Judged on the draft written, its team given or kept.
                coordinator_model_named(edited.fields().team.as_ref())?;
                Ok(vec![BoardArtifact::Plan(edited)])
            },
        )
        .map(written_plan);
        if let (Some(facts), Ok(plan)) = (facts, &written) {
            plan_text::note_written(facts, plan);
        }
        written
    }
}

/// The fields an edit gives, its text normalised: each `None` keeps the
/// draft's.
struct PlanPatch {
    approach: Option<String>,
    /// `Some(None)` takes the handoff away.
    handoff: Option<Option<String>>,
    items: Option<Vec<PlanItem>>,
    team: Option<Option<Team>>,
    entry: PlanEntry,
}

impl PlanPatch {
    /// The content of `held` with the given fields replaced whole.
    fn applied_to(&self, held: &PlanFields) -> PlanContent {
        PlanContent {
            approach: self
                .approach
                .clone()
                .unwrap_or_else(|| held.approach.clone()),
            handoff: self.handoff.clone().unwrap_or_else(|| held.handoff.clone()),
            items: self.items.clone().unwrap_or_else(|| held.items.clone()),
            team: self.team.clone().unwrap_or_else(|| held.team.clone()),
            entry: self.entry,
        }
    }

    /// Asserts that `edited` keeps every field the patch does not give
    /// exactly as `held` holds it.
    fn kept_in(&self, held: &PlanFields, edited: &PlanFields) {
        assert!(
            self.approach.is_some() || edited.approach == held.approach,
            "an edit keeps the approach it does not give"
        );
        assert!(
            self.handoff.is_some() || edited.handoff == held.handoff,
            "an edit keeps the handoff it does not give"
        );
        assert!(
            self.items.is_some() || edited.items == held.items,
            "an edit keeps the task items it does not give"
        );
        assert!(
            self.team.is_some() || edited.team == held.team,
            "an edit keeps the team it does not give"
        );
    }
}

#[cfg(test)]
#[path = "edit_plan_tests.rs"]
mod tests;
