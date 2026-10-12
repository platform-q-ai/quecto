//! What a use case reads off a board (#2481, M3): a task and its plans,
//! runs, changes and reviews. Each view refuses while a file in its scope
//! is set aside: what that file held is unknown, so an answer without it
//! could be wrong (a run's number reused, a change's pull request linked
//! twice). A file set aside whose name spells no artifact (a stray) is in
//! no view's scope, so it blocks none.
use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::{ArtifactKey, BoardSnapshot};
use crate::domain::project_board::entities::change::Change;
use crate::domain::project_board::entities::plan::Plan;
use crate::domain::project_board::entities::review::Review;
use crate::domain::project_board::entities::run::Run;
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::value_objects::slug::Slug;

/// The task `id`. Refused while its file is set aside, even if another
/// copy reads; [`ProjectBoardError::NotFound`] when the board holds none.
pub fn task<'b>(board: &'b BoardSnapshot, id: &Slug) -> Result<&'b Task, ProjectBoardError> {
    clear_of(board, |key| *key == ArtifactKey::Task(id.clone()))?;
    board
        .tasks
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
    let mut plans: Vec<&Plan> = board
        .plans
        .iter()
        .filter(|plan| plan.fields().task == *id)
        .collect();
    plans.sort_by_key(|plan| plan.fields().version);
    Ok(plans)
}

/// Every run of the task `id`, by number. Refused while any run of it is
/// set aside.
pub fn runs_of<'b>(board: &'b BoardSnapshot, id: &Slug) -> Result<Vec<&'b Run>, ProjectBoardError> {
    clear_of(
        board,
        |key| matches!(key, ArtifactKey::Run(task, _) if task == id),
    )?;
    let mut runs: Vec<&Run> = board
        .runs
        .iter()
        .filter(|run| run.fields().task == *id)
        .collect();
    runs.sort_by_key(|run| run.fields().number);
    Ok(runs)
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
    let mut changes: Vec<&Change> = board
        .changes
        .iter()
        .filter(|change| change.fields().task == *id)
        .collect();
    changes.sort_by_key(|change| change.fields().number);
    Ok(changes)
}

/// Every change on the board, by task and number: what a link checks a
/// pull request against, since one pull request is linked once board-wide.
/// Refused while any change of any task is set aside.
pub fn all_changes(board: &BoardSnapshot) -> Result<Vec<&Change>, ProjectBoardError> {
    clear_of(board, |key| matches!(key, ArtifactKey::Change(..)))?;
    let mut changes: Vec<&Change> = board.changes.iter().collect();
    changes.sort_by(|a, b| {
        (&a.fields().task, a.fields().number).cmp(&(&b.fields().task, b.fields().number))
    });
    Ok(changes)
}

/// The review of change `change` of the task `id`, if it has one. Refused
/// while that review's file is set aside.
pub fn review_of<'b>(
    board: &'b BoardSnapshot,
    id: &Slug,
    change: u32,
) -> Result<Option<&'b Review>, ProjectBoardError> {
    clear_of(board, |key| *key == ArtifactKey::Review(id.clone(), change))?;
    Ok(board
        .reviews
        .iter()
        .find(|review| review.fields().task == *id && review.fields().change == change))
}

/// Refuses with the least key in scope that a set-aside file names, so a
/// refusal is the same however the store orders the files.
fn clear_of(
    board: &BoardSnapshot,
    in_scope: impl Fn(&ArtifactKey) -> bool,
) -> Result<(), ProjectBoardError> {
    let blocking = board
        .quarantined
        .iter()
        .filter_map(|file| file.key.as_ref())
        .filter(|key| in_scope(key))
        .min();
    match blocking {
        Some(key) => Err(ProjectBoardError::Quarantined(key.clone())),
        None => Ok(()),
    }
}

#[cfg(test)]
#[path = "board_view_tests.rs"]
mod tests;
