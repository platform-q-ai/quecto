//! Whether an artifact is as its caller read it (#2705 B9): the board
//! orders its changes by revision, so a caller pins a change to the
//! revision it read at, `rev`, which `show` answers. The pin holds while
//! the artifact is as the board held it at that revision, whatever else
//! the board took since: at the very revision the change builds on, or
//! when the store held the same artifact then. A rev the board never held
//! never holds. The deprecated `updated` pin, taken for one more release,
//! holds while the artifact's stamp is the one given.
use crate::application::project_board::dto::{ProjectBoardError, ReadPin};
use crate::application::project_board::ports::{BoardArtifact, BoardSnapshot, ProjectBoardStore};
use crate::domain::project_board::entities::plan::Plan;
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::timestamp::Timestamp;

/// Whether `current`, as `board` holds it, stamped `updated`, is as the
/// caller read it, pinned by `pin`.
fn holds(
    store: &dyn ProjectBoardStore,
    board: &BoardSnapshot,
    current: &BoardArtifact,
    updated: &Timestamp,
    pin: &ReadPin,
) -> Result<bool, ProjectBoardError> {
    let key = current.key();
    assert_eq!(
        board.artifact(&key).as_ref(),
        Some(current),
        "a pin is judged against the artifact the board holds"
    );
    match pin {
        ReadPin::Rev(rev) if *rev == board.head => Ok(true),
        ReadPin::Rev(rev) => Ok(store.artifact_at(rev, &key)?.as_ref() == Some(current)),
        ReadPin::Updated(expected) => Ok(expected == updated),
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
    let Some(pin) = pin else {
        return Ok(());
    };
    let updated = &task.fields().updated;
    let current = BoardArtifact::Task(task.clone());
    match holds(store, board, &current, updated, pin)? {
        true => Ok(()),
        false => Err(ProjectBoardError::Stale {
            id: task.fields().id.clone(),
            expected: pin.clone(),
            updated: updated.clone(),
        }),
    }
}

/// `plan` on `board` as its caller read it, when `pin` is a rev pin;
/// refused as `plan_changed` otherwise. The deprecated `updated` pin is
/// the plan rules' own to judge (`plans::edit_draft` and its kin take
/// [`ReadPin::deprecated_updated`]), so it passes here.
///
/// # Errors
/// The plan changed since the pinned read; the store's failure to read
/// the pinned revision.
pub(super) fn plan_as_read(
    store: &dyn ProjectBoardStore,
    board: &BoardSnapshot,
    plan: &Plan,
    pin: Option<&ReadPin>,
) -> Result<(), ProjectBoardError> {
    let pin = match pin {
        Some(pin @ ReadPin::Rev(_)) => pin,
        Some(ReadPin::Updated(_)) | None => return Ok(()),
    };
    let updated = &plan.fields().updated;
    let current = BoardArtifact::Plan(plan.clone());
    match holds(store, board, &current, updated, pin)? {
        true => Ok(()),
        false => Err(ProjectBoardError::Rule(BoardRuleError::PlanChanged {
            version: plan.fields().version,
            updated: updated.clone(),
        })),
    }
}

#[cfg(test)]
#[path = "read_pins_tests.rs"]
mod tests;
