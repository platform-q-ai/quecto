//! What a use case reads off a board (#2481, M3): a task and its plans,
//! runs, changes and reviews, and what names a task's id. Each view
//! refuses while a file in its scope is set aside: what that file held is
//! unknown, so an answer without it
//! could be wrong (a run's number reused, a change's pull request linked
//! twice). A file set aside whose name spells no artifact (a stray) is in
//! no view's scope, so it blocks none. A view sees keys only: a kept file
//! set aside with no key (its artifact unknown) blocks no view, so a link
//! through [`all_changes`] still refuses while one exists
//! (`change_board_task`'s `links_readable` runs first). An artifact a
//! view finds more than once (a store that broke its contract of one
//! artifact per key) is refused as [`ProjectBoardError::Duplicate`], never
//! a panic.
use crate::application::project_board::dto::{IdUser, ProjectBoardError, once};
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
    let found: Vec<&Task> = tasks
        .iter()
        .filter(|task| task.fields().id == *id)
        .collect();
    match once(found, ArtifactKey::of_task)?.as_slice() {
        [task] => Ok(task),
        _ => Err(ProjectBoardError::NotFound(id.clone())),
    }
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
    once(
        artifacts_of::plans_of(&board.plans, id),
        ArtifactKey::of_plan,
    )
}

/// Every run of the task `id`, by number. Refused while any run of it is
/// set aside.
pub fn runs_of<'b>(board: &'b BoardSnapshot, id: &Slug) -> Result<Vec<&'b Run>, ProjectBoardError> {
    clear_of(
        board,
        |key| matches!(key, ArtifactKey::Run(task, _) if task == id),
    )?;
    once(artifacts_of::runs_of(&board.runs, id), ArtifactKey::of_run)
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
    once(
        artifacts_of::changes_of(&board.changes, id),
        ArtifactKey::of_change,
    )
}

/// Every change on the board, by task and number: what a link checks a
/// pull request against, since one pull request is linked once board-wide.
/// Refused while any change of any task is set aside.
pub fn all_changes(board: &BoardSnapshot) -> Result<Vec<&Change>, ProjectBoardError> {
    clear_of(board, |key| matches!(key, ArtifactKey::Change(..)))?;
    once(
        artifacts_of::all_changes(&board.changes),
        ArtifactKey::of_change,
    )
}

/// The review of change `change` of the task `id`, if it has one. Refused
/// while that review's file is set aside.
pub fn review_of<'b>(
    board: &'b BoardSnapshot,
    id: &Slug,
    change: u32,
) -> Result<Option<&'b Review>, ProjectBoardError> {
    clear_of(board, |key| *key == ArtifactKey::Review(id.clone(), change))?;
    let reviews = artifacts_of::reviews_of_change(&board.reviews, id, change);
    let reviews = once(reviews, ArtifactKey::of_review)?;
    Ok(reviews.first().copied())
}

/// The least artifact besides a task that names the task `id` (a plan,
/// run, change or review of it, or a discussion about it), or `None`: a
/// new task by that id would take over what it says. Refused while a
/// plan, run, change or review of it is set aside, or any discussion: the
/// file may name it too, and a set-aside discussion's task is unknown.
/// Creates are rare, so waiting for such a file to be mended costs little,
/// where reusing an id would join the new task to what an old one left.
pub fn naming(board: &BoardSnapshot, id: &Slug) -> Result<Option<IdUser>, ProjectBoardError> {
    clear_of(board, |key| match key {
        ArtifactKey::Plan(task, _)
        | ArtifactKey::Run(task, _)
        | ArtifactKey::Change(task, _)
        | ArtifactKey::Review(task, _) => task == id,
        ArtifactKey::Discussion(_) => true,
        ArtifactKey::Task(_) => false,
    })?;
    let plans = artifacts_of::plans_of(&board.plans, id);
    let runs = artifacts_of::runs_of(&board.runs, id);
    let changes = artifacts_of::changes_of(&board.changes, id);
    let reviews = board
        .reviews
        .iter()
        .filter(|review| review.fields().task == *id);
    let discussions = artifacts_of::discussions_of(&board.discussions, id);
    // The store's order is its own (reviews come unsorted): the least.
    let named = plans
        .into_iter()
        .map(|plan| IdUser::Plan(plan.fields().version))
        .chain(runs.into_iter().map(|run| IdUser::Run(run.fields().number)))
        .chain(
            changes
                .into_iter()
                .map(|change| IdUser::Change(change.fields().number)),
        )
        .chain(reviews.map(|review| IdUser::Review(review.fields().change)))
        .chain(
            discussions
                .into_iter()
                .map(|discussion| IdUser::Discussion(discussion.fields().id.clone())),
        )
        .min();
    Ok(named)
}

/// No task file is set aside (#2661): an archive waits, as one may be a
/// child of the task archived.
///
/// # Errors
/// The least task a set-aside file names.
pub(super) fn tasks_clear(board: &BoardSnapshot) -> Result<(), ProjectBoardError> {
    clear_of(board, |key| matches!(key, ArtifactKey::Task(_)))
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
        .filter_map(|file| file.key.as_ref().map(|key| (key, &file.path, &file.object)))
        .filter(|(key, ..)| in_scope(key))
        .min();
    match blocking {
        Some((key, path, object)) => Err(ProjectBoardError::Quarantined {
            key: key.clone(),
            path: path.clone(),
            object: object.clone(),
        }),
        None => Ok(()),
    }
}

#[cfg(test)]
#[path = "board_view_tests.rs"]
mod tests;
