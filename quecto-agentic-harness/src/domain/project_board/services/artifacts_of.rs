//! Which of a board's artifacts belong to a task, in their order (#2481,
//! M3): a task's plans by version, its runs (or one plan's) and changes by
//! number, a change's review, and the discussions linked to it by id. An
//! artifact pointing at what the board does not hold is still one of
//! them: each file is judged on its own.
use crate::domain::project_board::entities::change::Change;
use crate::domain::project_board::entities::discussion::Discussion;
use crate::domain::project_board::entities::plan::Plan;
use crate::domain::project_board::entities::review::Review;
use crate::domain::project_board::entities::run::Run;
use crate::domain::project_board::value_objects::slug::Slug;

/// Every plan of `task`, by version.
pub fn plans_of<'a>(plans: &'a [Plan], task: &Slug) -> Vec<&'a Plan> {
    sorted(
        plans.iter().filter(|plan| plan.fields().task == *task),
        |plan| plan.fields().version,
    )
}

/// Every run of `task`, by number.
pub fn runs_of<'a>(runs: &'a [Run], task: &Slug) -> Vec<&'a Run> {
    sorted(
        runs.iter().filter(|run| run.fields().task == *task),
        |run| run.fields().number,
    )
}

/// Every run of `task` of its plan's `version`, by number.
pub fn runs_of_plan<'a>(runs: &'a [Run], task: &Slug, version: u32) -> Vec<&'a Run> {
    sorted(
        runs.iter()
            .filter(|run| run.fields().task == *task && run.fields().plan_version == version),
        |run| run.fields().number,
    )
}

/// Every change of `task`, by number.
pub fn changes_of<'a>(changes: &'a [Change], task: &Slug) -> Vec<&'a Change> {
    sorted(
        changes
            .iter()
            .filter(|change| change.fields().task == *task),
        |change| change.fields().number,
    )
}

/// Every change, by task and number.
pub fn all_changes(changes: &[Change]) -> Vec<&Change> {
    sorted(changes.iter(), |change| {
        (change.fields().task.clone(), change.fields().number)
    })
}

/// The review of `task`'s change `change`, if there is one. A review's
/// file is named by its task and change, so there is at most one; were
/// there more, the first would be taken (and a debug build stops).
pub fn review_of<'a>(reviews: &'a [Review], task: &Slug, change: u32) -> Option<&'a Review> {
    let mut found = reviews
        .iter()
        .filter(|review| review.fields().task == *task && review.fields().change == change);
    let first = found.next();
    debug_assert!(found.next().is_none(), "one review per task and change");
    first
}

/// Every discussion linked to `task`, by id.
pub fn discussions_of<'a>(discussions: &'a [Discussion], task: &Slug) -> Vec<&'a Discussion> {
    sorted(
        discussions
            .iter()
            .filter(|discussion| discussion.fields().task.as_ref() == Some(task)),
        |discussion| discussion.fields().id.clone(),
    )
}

fn sorted<'a, T, K: Ord>(
    items: impl Iterator<Item = &'a T>,
    order: impl Fn(&T) -> K,
) -> Vec<&'a T> {
    let mut items: Vec<&T> = items.collect();
    items.sort_by_key(|item| order(item));
    items
}

#[cfg(test)]
#[path = "artifacts_of_tests.rs"]
mod tests;
