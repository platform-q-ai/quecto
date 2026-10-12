//! Each pick holds whatever order a store reads in (the in-memory store
//! reads in key order, so its tests cannot show this) and keeps only its
//! task's (and plan's, change's) artifacts. Two reviews of one change
//! break the stores' contract (one artifact per key); the pick returns
//! both, never panicking, so a caller can refuse a broken board.
use super::*;
use crate::domain::project_board::entities::discussion::{DiscussionFields, Message};
use crate::domain::project_board::entities::fixtures_tests::{
    at, change, open_run, person, plan, slug,
};
use crate::domain::project_board::entities::review::{ReviewFields, Round};
use crate::domain::project_board::entities::run::RunFields;
use crate::domain::project_board::value_objects::vocabulary::{
    DiscussionState, PlanState, ReviewVerdict,
};

/// The fixture task's discussion `id`, or another task's.
fn discussion(id: &str, task: &str) -> Discussion {
    Discussion::new(DiscussionFields {
        id: slug(id),
        title: "Locks".into(),
        task: Some(slug(task)),
        state: DiscussionState::Open,
        started_by: person("Ada"),
        messages: vec![Message {
            author: person("Ada"),
            at: at("09:00:00"),
            text: "Lease?".into(),
        }],
        created: at("09:00:00"),
        updated: at("09:00:00"),
    })
    .unwrap()
}

fn review(change: u32, verdict: ReviewVerdict) -> Review {
    Review::new(ReviewFields {
        task: slug("board-store"),
        change,
        rounds: vec![Round {
            commit: crate::domain::project_board::entities::fixtures_tests::commit('a'),
            reviewer: person("Bob"),
            at: at("09:10:00"),
            verdict,
            findings: vec![],
        }],
        created: at("09:10:00"),
        updated: at("09:10:00"),
    })
    .unwrap()
}

/// Run `number` of another task.
fn elsewhere(number: u32) -> Run {
    Run::new(RunFields {
        task: slug("other"),
        ..open_run(number, 1).into_fields()
    })
    .unwrap()
}

fn numbers<T>(items: Vec<&T>, number: impl Fn(&T) -> u32) -> Vec<u32> {
    items.into_iter().map(number).collect()
}

#[test]
fn each_pick_is_its_tasks_in_order_whatever_the_stores_order() {
    let task = slug("board-store");
    let plans = [
        plan(3, PlanState::Draft),
        plan(1, PlanState::Superseded),
        plan(2, PlanState::Accepted),
    ];
    let version = |plan: &Plan| plan.fields().version;
    assert_eq!(numbers(plans_of(&plans, &task), version), [1, 2, 3]);
    assert!(plans_of(&plans, &slug("other")).is_empty());
    let runs = [open_run(3, 1), open_run(2, 2), open_run(1, 1), elsewhere(4)];
    let number = |run: &Run| run.fields().number;
    assert_eq!(numbers(runs_of(&runs, &task), number), [1, 2, 3]);
    assert_eq!(
        numbers(of_plan_version(runs_of(&runs, &task), 1), number),
        [1, 3]
    );
    assert_eq!(
        numbers(of_plan_version(runs_of(&runs, &task), 2), number),
        [2]
    );
    assert!(of_plan_version(runs_of(&runs, &task), 9).is_empty());
    let changes = [change(2, None), change(1, None)];
    let number = |change: &Change| change.fields().number;
    assert_eq!(numbers(changes_of(&changes, &task), number), [1, 2]);
    assert_eq!(numbers(all_changes(&changes), number), [1, 2]);
    let discussions = [
        discussion("z", "board-store"),
        discussion("m", "board-store"),
        discussion("o", "other"),
    ];
    let ids: Vec<&str> = discussions_of(&discussions, &task)
        .into_iter()
        .map(|discussion| discussion.fields().id.as_str())
        .collect();
    assert_eq!(ids, ["m", "z"]);
}

#[test]
fn a_changes_review_is_found_by_its_task_and_change() {
    let task = slug("board-store");
    let reviews = [
        review(2, ReviewVerdict::ChangesRequested),
        review(1, ReviewVerdict::Approved),
    ];
    let found: Vec<ReviewVerdict> = reviews_of_change(&reviews, &task, 1)
        .into_iter()
        .map(Review::state)
        .collect();
    assert_eq!(found, [ReviewVerdict::Approved]);
    assert!(reviews_of_change(&reviews, &task, 3).is_empty());
    assert!(reviews_of_change(&reviews, &slug("other"), 1).is_empty());
}

/// Two reviews of one change cannot come from a store that keeps its
/// contract; the pick returns both rather than choose or panic.
#[test]
fn two_reviews_of_one_change_are_both_returned() {
    let reviews = [
        review(1, ReviewVerdict::Approved),
        review(1, ReviewVerdict::ChangesRequested),
    ];
    assert_eq!(
        reviews_of_change(&reviews, &slug("board-store"), 1).len(),
        2
    );
}
