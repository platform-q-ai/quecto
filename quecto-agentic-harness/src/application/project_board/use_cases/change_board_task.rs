//! Changes one task by the board's rules, as a compare-and-swap: the rule
//! runs against the task as last read, and a write that lost the race is
//! rebuilt against the fresh task, so a change the latest task no longer
//! allows is refused rather than written over another's. The actor is the
//! committer the store writes as, checked when the use case is built: no
//! caller acts as someone else. Blocking: a lost write pauses the calling
//! thread before it is tried again.
use std::sync::Arc;

use super::board_change::{checked_writer, commit, owned, task_on, written_task};
use super::board_view;
use crate::application::project_board::dto::{BoardOp, ProjectBoardError};
use crate::application::project_board::ports::{
    BoardArtifact, BoardClock, BoardSnapshot, ProjectBoardStore,
};
use crate::application::project_board::telemetry::OpFacts;
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::services::changes::Linked;
use crate::domain::project_board::services::claims::{claim_facts, renew, take_over};
use crate::domain::project_board::services::pull_requests::{add_pr, remove_pr};
use crate::domain::project_board::services::transitions::{move_reads, move_task};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;

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
    /// No such task, or its file quarantined; a move while a run or
    /// change of the task is set aside; a pull request linked while a file
    /// every write carries is set aside; the board's rules (against the
    /// latest task, its runs and changes, and every change on the board);
    /// the clock's; a board that kept moving; the store's.
    pub fn execute(&self, id: &Slug, op: &BoardOp) -> Result<Task, ProjectBoardError> {
        self.execute_noted(id, op, None)
    }

    /// [`Self::execute`], noting in `facts` (epic #2481 T4), when given,
    /// the write's attempts and, of the task as last read, its status and
    /// the claim it held; the status a move asked for, made or refused,
    /// and otherwise the written task's.
    ///
    /// # Errors
    /// As [`Self::execute`].
    pub fn execute_noted(
        &self,
        id: &Slug,
        op: &BoardOp,
        mut facts: Option<&mut OpFacts>,
    ) -> Result<Task, ProjectBoardError> {
        let actor = &self.actor;
        let noting = facts.is_some();
        let mut read = None;
        let noted = facts.as_deref_mut();
        let written = commit(
            &*self.store,
            &*self.clock,
            &op.message(id),
            noted,
            |board, when| {
                let task = task_on(board, id);
                if noting {
                    // This attempt's read, or none: never an earlier one's.
                    let found = task.as_ref().ok();
                    read = found
                        .map(|task| (task.fields().status, claim_facts(task, actor, &when.now)));
                }
                let task = task?;
                when.may_build_on(task)?;
                board_readable_for(op, board)?;
                let changed = apply(task, board, actor, op, &when.now)?;
                Ok(vec![BoardArtifact::Task(changed)])
            },
        );
        let written = written.map(written_task);
        if let Some(facts) = facts {
            if let Some((status, claim)) = read {
                facts.status_from = Some(status);
                facts.claim = claim;
            }
            facts.status_to = match op {
                BoardOp::Move(to) => Some(*to),
                BoardOp::Renew | BoardOp::TakeOver | BoardOp::AddPr(_) | BoardOp::RemovePr(_) => {
                    written.as_ref().ok().map(|task| task.fields().status)
                }
            };
        }
        written
    }
}

/// The board holds what `op` must see: linking a pull request needs every
/// task's, so while a file every write carries is set aside (what it holds
/// unknown, whether or not its task can be told) a link is refused rather
/// than risk linking one twice. A stray the next write drops holds nothing
/// that stays, so it blocks nothing. The other ops read only their own
/// task. Of several such files the least is named (one whose artifact is
/// unknown first, then by key and path), whatever the store's order.
/// It runs before [`board_view::all_changes`], which sees only keys: a
/// kept file with no key blocks no view, so this is what keeps a link
/// failing closed on one.
fn board_readable_for(op: &BoardOp, board: &BoardSnapshot) -> Result<(), ProjectBoardError> {
    match op {
        BoardOp::AddPr(_) => {
            let carried = board
                .quarantined
                .iter()
                .filter(|file| file.carried)
                .min_by(|a, b| (&a.key, &a.path, &a.object).cmp(&(&b.key, &b.path, &b.object)));
            match carried {
                None => Ok(()),
                Some(file) => match &file.key {
                    Some(key) => Err(ProjectBoardError::Quarantined {
                        key: key.clone(),
                        path: file.path.clone(),
                        object: file.object.clone(),
                    }),
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

/// `op` on `task`, as `board` holds it. A move reads the task's runs and
/// changes only as the rule names them ([`move_reads`]: the use case keeps
/// no copy of the table): a run going or a change linked holds the task, a
/// move that reads a set-aside one waits, and a move the rule refuses
/// anyway is refused as such, reading nothing. A link takes every change
/// on the board and every task, so a pull request is linked once
/// whichever way.
fn apply(
    task: &Task,
    board: &BoardSnapshot,
    actor: &Identity,
    op: &BoardOp,
    now: &Timestamp,
) -> Result<Task, ProjectBoardError> {
    let id = &task.fields().id;
    let ruled = match op {
        BoardOp::Renew => renew(task, actor, now),
        BoardOp::TakeOver => take_over(task, actor, now),
        BoardOp::Move(to) => {
            let reads = move_reads(task, *to, actor, now)?;
            let runs = match reads.runs {
                true => owned(board_view::runs_of(board, id)?),
                false => vec![],
            };
            let changes = match reads.changes {
                true => owned(board_view::changes_of(board, id)?),
                false => vec![],
            };
            // The tasks it relates to may hold the move back (#2661).
            let moved = move_task(task, *to, actor, (&runs, &changes), &board.tasks, now)?;
            // Once the rules allow it, a task file set aside may still be a
            // child an archive waits for.
            if *to == TaskStatus::Archived {
                board_view::tasks_clear(board)?;
            }
            Ok(moved)
        }
        BoardOp::AddPr(pr) => {
            let changes = owned(board_view::all_changes(board)?);
            let linked = Linked {
                changes: &changes,
                tasks: &board.tasks,
            };
            add_pr(task, actor, *pr, linked, now)
        }
        BoardOp::RemovePr(pr) => remove_pr(task, actor, *pr, now),
    };
    ruled.map_err(ProjectBoardError::Rule)
}

#[cfg(test)]
#[path = "change_board_task_tests.rs"]
mod tests;
