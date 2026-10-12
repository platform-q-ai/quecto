//! A task's plans, runs, changes, review and discussions, picked from one
//! read of the board (#2481, M3f). `QueryBoard` and the index's listings
//! both answer from here until the index keeps a table per kind (S12),
//! when the index answers in SQL and its contract holds it to these.
use super::dto::ArtifactListing;
use super::ports::{BoardHead, BoardSnapshot, QuarantinedFile};
use crate::domain::project_board::entities::change::Change;
use crate::domain::project_board::entities::discussion::Discussion;
use crate::domain::project_board::entities::plan::Plan;
use crate::domain::project_board::entities::review::Review;
use crate::domain::project_board::entities::run::Run;
use crate::domain::project_board::value_objects::slug::Slug;

/// Every plan of `task`, by version.
pub fn plans_of(board: BoardSnapshot, task: &Slug) -> ArtifactListing<Plan> {
    picked(
        board.head,
        board.quarantined,
        board.plans,
        |plan| plan.fields().task == *task,
        |plan| plan.fields().version,
    )
}

/// Every run of `task` of its plan's `version`, by number.
pub fn runs_of_plan(board: BoardSnapshot, task: &Slug, version: u32) -> ArtifactListing<Run> {
    picked(
        board.head,
        board.quarantined,
        board.runs,
        |run| run.fields().task == *task && run.fields().plan_version == version,
        |run| run.fields().number,
    )
}

/// Every change of `task`, by number.
pub fn changes_of(board: BoardSnapshot, task: &Slug) -> ArtifactListing<Change> {
    picked(
        board.head,
        board.quarantined,
        board.changes,
        |change| change.fields().task == *task,
        |change| change.fields().number,
    )
}

/// The review of `task`'s change `change`: none or one, as a review's
/// file is named by its task and change.
pub fn review_of(board: BoardSnapshot, task: &Slug, change: u32) -> ArtifactListing<Review> {
    let listing = picked(
        board.head,
        board.quarantined,
        board.reviews,
        |review| review.fields().task == *task && review.fields().change == change,
        |review| review.fields().change,
    );
    assert!(
        listing.items.len() <= 1,
        "a board holds one review per task and change"
    );
    listing
}

/// Every discussion linked to `task`, by id.
pub fn discussions_of(board: BoardSnapshot, task: &Slug) -> ArtifactListing<Discussion> {
    picked(
        board.head,
        board.quarantined,
        board.discussions,
        |discussion| discussion.fields().task.as_ref() == Some(task),
        |discussion| discussion.fields().id.clone(),
    )
}

/// The `items` `wanted` keeps, ordered by `order`, beside every file set
/// aside.
fn picked<T, K: Ord>(
    head: BoardHead,
    quarantined: Vec<QuarantinedFile>,
    items: Vec<T>,
    wanted: impl Fn(&T) -> bool,
    order: impl Fn(&T) -> K,
) -> ArtifactListing<T> {
    let mut items: Vec<T> = items.into_iter().filter(|item| wanted(item)).collect();
    items.sort_by_key(|item| order(item));
    ArtifactListing {
        head,
        items,
        quarantined,
    }
}

#[cfg(test)]
#[path = "artifact_listings_tests.rs"]
mod tests;
