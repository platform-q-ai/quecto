//! What a use case reads off a board (#2481, M3): a task and its plans,
//! runs, changes and reviews. Each view refuses while a file in its scope
//! is set aside: what that file held is unknown, so an answer without it
//! could be wrong (a run's number reused, a change's pull request linked
//! twice). A file set aside whose name spells no artifact (a stray) is in
//! no view's scope, so it blocks none. A view sees keys only: a kept file
//! set aside with no key (its artifact unknown) blocks no view, so S7's
//! link wiring through [`all_changes`] must still refuse while one exists,
//! as `change_board_task`'s `board_readable_for` does now.
use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::{ArtifactKey, BoardSnapshot, QuarantinedFile};
use crate::domain::project_board::entities::change::Change;
use crate::domain::project_board::entities::plan::Plan;
use crate::domain::project_board::entities::review::Review;
use crate::domain::project_board::entities::run::Run;
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::services::artifacts_of;
use crate::domain::project_board::value_objects::slug::Slug;

/// The task `id`. Refused while its file is set aside, even if another
/// copy reads (no store answers both: each checks it as it reads);
/// [`ProjectBoardError::NotFound`] when the board holds none. Every use
/// case asking whether a task is there asks here, so all answer alike.
pub fn task<'b>(board: &'b BoardSnapshot, id: &Slug) -> Result<&'b Task, ProjectBoardError> {
    task_in(&board.tasks, &board.quarantined, id)
}

/// [`task`] over a listing's `tasks` and the files it set aside.
pub fn task_in<'b>(
    tasks: &'b [Task],
    quarantined: &[QuarantinedFile],
    id: &Slug,
) -> Result<&'b Task, ProjectBoardError> {
    first_set_aside(quarantined, |key| *key == ArtifactKey::Task(id.clone()))?;
    tasks
        .iter()
        .find(|task| task.fields().id == *id)
        .ok_or_else(|| ProjectBoardError::NotFound(id.clone()))
}

/// Every plan of the task `id`, by version. Refused while any plan of it
/// is set aside.
pub fn plans_of<'b>(
    board: &'b BoardSnapshot,
    id: &Slug,
) -> Result<Vec<&'b Plan>, ProjectBoardError> {
    clear_of(
        board,
        |key| matches!(key, ArtifactKey::Plan(task, _) if task == id),
    )?;
    Ok(artifacts_of::plans_of(&board.plans, id))
}

/// Every run of the task `id`, by number. Refused while any run of it is
/// set aside.
pub fn runs_of<'b>(board: &'b BoardSnapshot, id: &Slug) -> Result<Vec<&'b Run>, ProjectBoardError> {
    clear_of(
        board,
        |key| matches!(key, ArtifactKey::Run(task, _) if task == id),
    )?;
    Ok(artifacts_of::runs_of(&board.runs, id))
}

/// Every change of the task `id`, by number. Refused while any change of
/// it is set aside.
pub fn changes_of<'b>(
    board: &'b BoardSnapshot,
    id: &Slug,
) -> Result<Vec<&'b Change>, ProjectBoardError> {
    clear_of(
        board,
        |key| matches!(key, ArtifactKey::Change(task, _) if task == id),
    )?;
    Ok(artifacts_of::changes_of(&board.changes, id))
}

/// Every change on the board, by task and number: what a link checks a
/// pull request against, since one pull request is linked once board-wide.
/// Refused while any change of any task is set aside.
pub fn all_changes(board: &BoardSnapshot) -> Result<Vec<&Change>, ProjectBoardError> {
    clear_of(board, |key| matches!(key, ArtifactKey::Change(..)))?;
    Ok(artifacts_of::all_changes(&board.changes))
}

/// The review of change `change` of the task `id`, if it has one. Refused
/// while that review's file is set aside.
pub fn review_of<'b>(
    board: &'b BoardSnapshot,
    id: &Slug,
    change: u32,
) -> Result<Option<&'b Review>, ProjectBoardError> {
    clear_of(board, |key| *key == ArtifactKey::Review(id.clone(), change))?;
    Ok(artifacts_of::review_of(&board.reviews, id, change))
}

/// Refuses with the least key in scope that a set-aside file names, so a
/// refusal is the same however the store orders the files.
fn clear_of(
    board: &BoardSnapshot,
    in_scope: impl Fn(&ArtifactKey) -> bool,
) -> Result<(), ProjectBoardError> {
    first_set_aside(&board.quarantined, in_scope)
}

fn first_set_aside(
    quarantined: &[QuarantinedFile],
    in_scope: impl Fn(&ArtifactKey) -> bool,
) -> Result<(), ProjectBoardError> {
    let blocking = quarantined
        .iter()
        .filter_map(|file| file.key.as_ref().map(|key| (key, &file.path)))
        .filter(|(key, _)| in_scope(key))
        .min();
    match blocking {
        Some((key, path)) => Err(ProjectBoardError::Quarantined {
            key: key.clone(),
            path: path.clone(),
        }),
        None => Ok(()),
    }
}

#[cfg(test)]
#[path = "board_view_tests.rs"]
mod tests;
