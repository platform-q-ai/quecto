//! Changes one task by the board's rules, as a compare-and-swap: the rule
//! runs against the task as last read, and a write that lost the race is
//! rebuilt against the fresh task, so a change the latest task no longer
//! allows is refused rather than written over another's. The actor is the
//! committer the store writes as, checked when the use case is built: no
//! caller acts as someone else. Blocking: a lost write pauses the calling
//! thread before it is tried again.
use std::sync::Arc;

use super::board_change::{checked_writer, commit, task_on};
use crate::application::project_board::dto::{BoardOp, ProjectBoardError};
use crate::application::project_board::ports::{BoardClock, BoardSnapshot, ProjectBoardStore};
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::services::changes::Linked;
use crate::domain::project_board::services::claims::{renew, take_over};
use crate::domain::project_board::services::pull_requests::{add_pr, remove_pr};
use crate::domain::project_board::services::transitions::move_task;
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;

pub struct ChangeBoardTask {
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
    /// The store's committer, checked valid when built.
    actor: Identity,
}

impl ChangeBoardTask {
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

    /// Task `id` after `op`, made by the store's committer.
    ///
    /// # Errors
    /// No such task, or its file quarantined; a pull request linked while
    /// another task's file is set aside; the board's rules (against
    /// the latest task); the clock's; a board that kept moving; the
    /// store's.
    pub fn execute(&self, id: &Slug, op: &BoardOp) -> Result<Task, ProjectBoardError> {
        let actor = &self.actor;
        let tasks = commit(
            &*self.store,
            &*self.clock,
            &op.message(id),
            |board, when| {
                let task = task_on(board, id)?;
                when.may_build_on(task)?;
                board_readable_for(op, board)?;
                Ok(vec![
                    apply(task, &board.tasks, actor, op, &when.now)
                        .map_err(ProjectBoardError::Rule)?,
                ])
            },
        )?;
        Ok(tasks.into_iter().next().expect("a change writes its task"))
    }
}

/// The board holds what `op` must see: linking a pull request needs every
/// task's, so while a file every write carries is set aside (what it holds
/// unknown, whether or not its task can be told) a link is refused rather
/// than risk linking one twice. A stray the next write drops holds nothing
/// that stays, so it blocks nothing. The other ops read only their own
/// task.
fn board_readable_for(op: &BoardOp, board: &BoardSnapshot) -> Result<(), ProjectBoardError> {
    match op {
        BoardOp::AddPr(_) => {
            let carried = board.quarantined.iter().find(|file| file.carried);
            match carried {
                None => Ok(()),
                Some(file) => match &file.key {
                    Some(key) => Err(ProjectBoardError::Quarantined(key.clone())),
                    None => Err(ProjectBoardError::SetAsideFile {
                        path: file.path.clone(),
                        object: file.object.clone(),
                    }),
                },
            }
        }
        BoardOp::Renew | BoardOp::TakeOver | BoardOp::Move(_) | BoardOp::RemovePr(_) => Ok(()),
    }
}

/// `op` on `task`, one of the board's `tasks`.
fn apply(
    task: &Task,
    tasks: &[Task],
    actor: &Identity,
    op: &BoardOp,
    now: &Timestamp,
) -> Result<Task, BoardRuleError> {
    match op {
        BoardOp::Renew => renew(task, actor, now),
        BoardOp::TakeOver => take_over(task, actor, now),
        // No run or change is stored before the run and change artifacts
        // are (M3): the task has none, so none holds it in review, and no
        // change on the board links the pull request. From then the use
        // case passes the task's runs and changes, and to `add_pr` every
        // change on the board too. A link waits while a task's file is set
        // aside ([`board_readable_for`]).
        BoardOp::Move(to) => move_task(task, *to, actor, &[], &[], now),
        BoardOp::AddPr(pr) => {
            let board = Linked {
                changes: &[],
                tasks,
            };
            add_pr(task, actor, *pr, board, now)
        }
        BoardOp::RemovePr(pr) => remove_pr(task, actor, *pr, now),
    }
}

#[cfg(test)]
#[path = "change_board_task_tests.rs"]
mod tests;
