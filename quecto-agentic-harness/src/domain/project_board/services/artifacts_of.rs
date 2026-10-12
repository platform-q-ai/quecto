//! Which of a board's artifacts belong to a task, in their order (#2481,
//! M3): a task's plans by version, its runs (or one plan's) and changes by
//! number, a change's reviews, and the discussions linked to it by id. An
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
        |a, b| a.fields().version.cmp(&b.fields().version),
    )
}

/// Every run of `task`, by number.
pub fn runs_of<'a>(runs: &'a [Run], task: &Slug) -> Vec<&'a Run> {
    sorted(
        runs.iter().filter(|run| run.fields().task == *task),
        |a, b| a.fields().number.cmp(&b.fields().number),
    )
}

/// Those of `runs` (a task's, by number) of its plan's `version`, in
/// their order: narrowed after each is known to be held once, since a
/// copy of another version is still the same run.
pub fn of_plan_version(runs: Vec<&Run>, version: u32) -> Vec<&Run> {
    runs.into_iter()
        .filter(|run| run.fields().plan_version == version)
        .collect()
}

/// Every change of `task`, by number.
pub fn changes_of<'a>(changes: &'a [Change], task: &Slug) -> Vec<&'a Change> {
    sorted(
        changes
            .iter()
            .filter(|change| change.fields().task == *task),
        |a, b| a.fields().number.cmp(&b.fields().number),
    )
}

/// Every change, by task and number.
pub fn all_changes(changes: &[Change]) -> Vec<&Change> {
    sorted(changes.iter(), |a, b| {
        (&a.fields().task, a.fields().number).cmp(&(&b.fields().task, b.fields().number))
    })
}

/// The reviews of `task`'s change `change`: none or one from any store
/// that keeps its contract (one artifact per key); every one is returned,
/// so a caller can tell a broken board from a whole one.
pub fn reviews_of_change<'a>(reviews: &'a [Review], task: &Slug, change: u32) -> Vec<&'a Review> {
    reviews
        .iter()
        .filter(|review| review.fields().task == *task && review.fields().change == change)
        .collect()
}

/// Every discussion linked to `task`, by id.
pub fn discussions_of<'a>(discussions: &'a [Discussion], task: &Slug) -> Vec<&'a Discussion> {
    sorted(
        discussions
            .iter()
            .filter(|discussion| discussion.fields().task.as_ref() == Some(task)),
        |a, b| a.fields().id.cmp(&b.fields().id),
    )
}

fn sorted<'a, T>(
    items: impl Iterator<Item = &'a T>,
    order: impl Fn(&T, &T) -> std::cmp::Ordering,
) -> Vec<&'a T> {
    let mut items: Vec<&T> = items.collect();
    items.sort_by(|a, b| order(a, b));
    items
}

#[cfg(test)]
#[path = "artifacts_of_tests.rs"]
mod tests;
