use super::QueryBoard;
use crate::application::project_board::dto::BoardListing;
use crate::application::project_board::ports::ProjectBoardStore;
use crate::application::project_board::use_cases::fixtures_tests::{at, board, fields, slug};
use crate::domain::project_board::entities::claim::{Claim, Identity};
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;
use std::sync::Arc;

/// c (draft, child of a), b (ready, child of a, depends on a), a (ready),
/// d (done, depends on a and b), e (ready, depends on d), f (claimed by Ada
/// until 10:00), written out of order; g's file is quarantined.
fn query() -> QueryBoard {
    let store = board();
    let task = |fields: TaskFields| {
        Task::new(TaskFields {
            created: at("08:00:00"),
            updated: at("08:00:00"),
            ..fields
        })
        .unwrap()
    };
    let ada = Identity {
        name: "Ada".into(),
        email: "ada@example.com".into(),
    };
    let claim = Claim {
        holder: ada,
        since: at("08:00:00"),
        expires: at("10:00:00"),
    };
    let tasks = vec![
        task(TaskFields {
            status: TaskStatus::Draft,
            parent: Some(slug("a")),
            ..fields("c")
        }),
        task(TaskFields {
            parent: Some(slug("a")),
            depends_on: vec![slug("a")],
            ..fields("b")
        }),
        task(fields("a")),
        task(TaskFields {
            status: TaskStatus::Done,
            depends_on: vec![slug("a"), slug("b")],
            ..fields("d")
        }),
        task(TaskFields {
            depends_on: vec![slug("d")],
            ..fields("e")
        }),
        task(TaskFields {
            status: TaskStatus::Claimed,
            claim: Some(claim),
            ..fields("f")
        }),
    ];
    let head = store.read().unwrap().head;
    store.write(&head, &tasks, "seed").unwrap();
    store.quarantine("g", "malformed");
    QueryBoard::new(Arc::new(store))
}

fn ids(listing: BoardListing) -> Vec<String> {
    assert_eq!(
        listing.quarantined.len(),
        1,
        "every listing carries the quarantined files"
    );
    listing
        .tasks
        .iter()
        .map(|task| task.fields().id.as_str().to_owned())
        .collect()
}

#[test]
fn listings_answer_status_children_and_dependents_by_id() {
    let query = query();
    assert_eq!(
        ids(query.by_status(TaskStatus::Ready).unwrap()),
        ["a", "b", "e"]
    );
    assert_eq!(ids(query.by_status(TaskStatus::Draft).unwrap()), ["c"]);
    assert_eq!(
        ids(query.by_status(TaskStatus::Archived).unwrap()),
        Vec::<String>::new()
    );
    assert_eq!(ids(query.children(&slug("a")).unwrap()), ["b", "c"]);
    assert_eq!(ids(query.dependents(&slug("a")).unwrap()), ["b", "d"]);
    assert_eq!(ids(query.dependents(&slug("b")).unwrap()), ["d"]);
}

#[test]
fn ready_lists_only_tasks_whose_dependencies_are_done() {
    assert_eq!(
        ids(query().ready().unwrap()),
        ["a", "e"],
        "b waits on a, which is not done"
    );
}

#[test]
fn takeable_lists_held_tasks_whose_claim_expired() {
    let query = query();
    assert_eq!(
        ids(query.takeable(&at("09:59:59")).unwrap()),
        Vec::<String>::new()
    );
    assert_eq!(ids(query.takeable(&at("10:00:00")).unwrap()), ["f"]);
}
