//! The order of each listing holds whatever order a store reads in (the
//! in-memory store reads in key order, so its tests cannot show this).
//! The fixture runs are of plan 1.
use super::*;
use crate::domain::project_board::entities::discussion::DiscussionFields;
use crate::infrastructure::test_support::board_artifacts::{change, discussion, plan, run};

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

#[test]
fn each_listing_is_ordered_whatever_the_stores_order() {
    let on_a = |id: &str| {
        Discussion::new(DiscussionFields {
            task: Some(slug("a")),
            ..discussion(id).into_fields()
        })
        .unwrap()
    };
    let board = || BoardSnapshot {
        head: BoardHead("h".into()),
        tasks: vec![],
        plans: vec![plan("a", 3), plan("a", 1), plan("a", 2)],
        runs: vec![run("a", 3), run("a", 1), run("a", 2)],
        changes: vec![change("a", 3), change("a", 1), change("a", 2)],
        reviews: vec![],
        discussions: vec![on_a("z"), on_a("m"), on_a("q")],
        quarantined: vec![],
    };
    let a = slug("a");
    let versions: Vec<u32> = plans_of(board(), &a)
        .items
        .iter()
        .map(|plan| plan.fields().version)
        .collect();
    assert_eq!(versions, [1, 2, 3]);
    let runs: Vec<u32> = runs_of_plan(board(), &a, 1)
        .items
        .iter()
        .map(|run| run.fields().number)
        .collect();
    assert_eq!(runs, [1, 2, 3]);
    let changes: Vec<u32> = changes_of(board(), &a)
        .items
        .iter()
        .map(|change| change.fields().number)
        .collect();
    assert_eq!(changes, [1, 2, 3]);
    let listed = discussions_of(board(), &a);
    let ids: Vec<&str> = listed
        .items
        .iter()
        .map(|discussion| discussion.fields().id.as_str())
        .collect();
    assert_eq!(ids, ["m", "q", "z"]);
}
