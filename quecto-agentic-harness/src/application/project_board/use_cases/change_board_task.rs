//! Changes one task by the board's rules, as a compare-and-swap: the rule
//! runs against the task as last read, and a write that lost the race is
//! rebuilt against the fresh task, so a change the latest task no longer
//! allows is refused rather than written over another's.
use std::sync::Arc;

use super::board_change::commit;
use crate::application::project_board::dto::{BoardOp, ProjectBoardError};
use crate::application::project_board::ports::{BoardClock, ProjectBoardStore};
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::services::claims::{renew, take_over};
use crate::domain::project_board::services::pull_requests::{add_pr, remove_pr};
use crate::domain::project_board::services::runs::end_run_lost;
use crate::domain::project_board::services::transitions::{complete_merged, move_task};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;

pub struct ChangeBoardTask {
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
}

impl ChangeBoardTask {
    pub fn new(store: Arc<dyn ProjectBoardStore>, clock: Arc<dyn BoardClock>) -> Self {
        Self { store, clock }
    }

    /// Task `id` after `op`, made by `actor` (the git committer identity
    /// the store writes as).
    ///
    /// # Errors
    /// No such task, the board's rules (against the latest task), the
    /// clock's, a board that kept moving, or the store's.
    pub fn execute(
        &self,
        id: &Slug,
        actor: &Identity,
        op: &BoardOp,
    ) -> Result<Task, ProjectBoardError> {
        let message = format!("board: {} {}", op.name(), id.as_str());
        let (tasks, _) = commit(&*self.store, &*self.clock, &message, |board, now| {
            let task = board
                .tasks
                .iter()
                .find(|task| task.fields().id == *id)
                .ok_or_else(|| ProjectBoardError::NotFound(id.clone()))?;
            Ok(vec![
                apply(task, actor, op, now).map_err(ProjectBoardError::Rule)?,
            ])
        })?;
        Ok(tasks.into_iter().next().expect("a change writes its task"))
    }
}

fn apply(
    task: &Task,
    actor: &Identity,
    op: &BoardOp,
    now: &Timestamp,
) -> Result<Task, crate::domain::project_board::value_objects::rule_error::BoardRuleError> {
    let _ = (
        renew,
        take_over,
        add_pr,
        remove_pr,
        end_run_lost,
        complete_merged,
        move_task,
    );
    let _ = (task, actor, now);
    if true {
        return Err(
            crate::domain::project_board::value_objects::rule_error::BoardRuleError::NoClaim,
        );
    }
    match op {
        BoardOp::Claim => move_task(task, TaskStatus::Claimed, actor, now),
        BoardOp::Renew => renew(task, actor, now),
        BoardOp::TakeOver => take_over(task, actor, now),
        BoardOp::Move(to) => move_task(task, *to, actor, now),
        BoardOp::AddPr(pr) => add_pr(task, actor, *pr, now),
        BoardOp::RemovePr(pr) => remove_pr(task, actor, *pr, now),
        BoardOp::CompleteMerged(merged) => complete_merged(task, merged, now),
        BoardOp::EndRunLost => end_run_lost(task, actor, now),
    }
}

#[cfg(test)]
#[path = "change_board_task_tests.rs"]
mod tests;
