//! Whether an artifact is as its caller read it (#2705 B9): the board
//! orders its changes by revision, so a caller pins a change to the
//! revision it read at, `rev`, which `show` answers. The pin holds while
//! the artifact's file is as the board held it at that revision, whatever
//! else the board took since: at the very revision the change builds on,
//! or when the store names the same file at both ([`ProjectBoardStore::
//! object_at`]). A rev the board never held never holds. The deprecated
//! `updated` pin, taken for one more release, holds while the artifact's
//! stamp is the one given: it no longer tells apart two changes in one
//! second, so it may let a change overwrite a newer one.
use crate::application::project_board::dto::{ProjectBoardError, ReadPin};
use crate::application::project_board::ports::{
    ArtifactKey, BoardHead, BoardSnapshot, ProjectBoardStore,
};
use crate::domain::project_board::entities::plan::Plan;
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::services::plans::PlanRead;
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;

/// Whether the artifact `key` names on `board` is as it was at `rev`.
fn unchanged_since(
    store: &dyn ProjectBoardStore,
    board: &BoardSnapshot,
    key: &ArtifactKey,
    rev: &BoardHead,
) -> Result<bool, ProjectBoardError> {
    assert!(
        board.artifact(key).is_some(),
        "a pin is judged against an artifact the board holds"
    );
    unchanged_at(store, &board.head, key, rev)
}

/// Whether `key`'s file at `head`, which a read just found, is the one it
/// was at `rev`.
fn unchanged_at(
    store: &dyn ProjectBoardStore,
    head: &BoardHead,
    key: &ArtifactKey,
    rev: &BoardHead,
) -> Result<bool, ProjectBoardError> {
    if rev == head {
        return Ok(true);
    }
    let then = store.object_at(rev, key)?;
    let now = store.object_at(head, key)?;
    assert!(now.is_some(), "the board names the file it read");
    Ok(then.is_some() && then == now)
}

/// A page of `plan` (the one a `show` read at `head` gives; `None`: none)
/// as it was at `rev`, its first page's revision; refused as
/// `plan_changed` otherwise. A show with no plan to page holds any pin.
///
/// # Errors
/// The plan changed since; the store's failure to read `rev`.
pub(super) fn page_as_read(
    store: &dyn ProjectBoardStore,
    head: &BoardHead,
    plan: Option<&Plan>,
    rev: &BoardHead,
) -> Result<(), ProjectBoardError> {
    let Some(plan) = plan else {
        return Ok(());
    };
    match unchanged_at(store, head, &ArtifactKey::of_plan(plan), rev)? {
        true => Ok(()),
        false => Err(ProjectBoardError::Rule(BoardRuleError::PlanChanged {
            version: plan.fields().version,
            updated: plan.fields().updated.clone(),
        })),
    }
}

/// `task` on `board` as its caller read it, when `pin` is given; refused
/// as stale otherwise.
///
/// # Errors
/// The task changed since the pinned read; the store's failure to read
/// the pinned revision.
pub(super) fn task_as_read(
    store: &dyn ProjectBoardStore,
    board: &BoardSnapshot,
    task: &Task,
    pin: Option<&ReadPin>,
) -> Result<(), ProjectBoardError> {
    let updated = &task.fields().updated;
    let holds = match pin {
        None => true,
        Some(ReadPin::Rev(rev)) => unchanged_since(store, board, &ArtifactKey::of_task(task), rev)?,
        Some(ReadPin::Updated(expected)) => expected == updated,
    };
    match (holds, pin) {
        (true, _) | (false, None) => Ok(()),
        (false, Some(pin)) => Err(ProjectBoardError::Stale {
            id: task.fields().id.clone(),
            expected: pin.clone(),
            updated: updated.clone(),
        }),
    }
}

/// How `pin` pins `plan` on `board`, as the plan rules take it: a rev
/// pin's verdict, judged here (the rules cannot read the board's history)
/// and told by the rules where they check a pin, after the draft and its
/// state (#2705 B9 review round 1: a retried acceptance whose first try
/// landed is told the plan is no draft); the deprecated stamp pin as it is.
///
/// # Errors
/// The store's failure to read the pinned revision.
pub(super) fn plan_read<'p>(
    store: &dyn ProjectBoardStore,
    board: &BoardSnapshot,
    plan: Option<&Plan>,
    pin: Option<&'p ReadPin>,
) -> Result<PlanRead<'p>, ProjectBoardError> {
    match (pin, plan) {
        (None, _) => Ok(PlanRead::Unpinned),
        (Some(ReadPin::Updated(expected)), _) => Ok(PlanRead::Stamped(expected)),
        // No such plan: the rules refuse it before any pin.
        (Some(ReadPin::Rev(_)), None) => Ok(PlanRead::Unpinned),
        (Some(ReadPin::Rev(rev)), Some(plan)) => {
            match unchanged_since(store, board, &ArtifactKey::of_plan(plan), rev)? {
                true => Ok(PlanRead::Unchanged),
                false => Ok(PlanRead::Changed),
            }
        }
    }
}

#[cfg(test)]
#[path = "read_pins_tests.rs"]
mod tests;
