use super::QueryBoard;
use crate::application::project_board::dto::BoardListing;
use crate::application::project_board::ports::ProjectBoardStore;
use crate::application::project_board::use_cases::fixtures_tests::{
    at, board, fields, finished, slug,
};
use crate::domain::project_board::entities::claim::{Claim, Identity};
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::services::transitions::move_task;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use std::sync::Arc;

/// c (draft, child of a), b (ready, child of a, depends on a), a (ready),
/// d (done, depends on a and b), e (ready, depends on d), f (claimed by Ada
/// until 10:00), written out of order; g's file is quarantined.
fn query() -> QueryBoard {
    QueryBoard::new(Arc::new(seeded()))
}

/// The board [`query`] reads.
fn seeded() -> MemoryBoardStore {
    let store = board();
    let task = |fields: TaskFields| {
        Task::new(finished(TaskFields {
            created: at("08:00:00"),
            updated: at("08:00:00"),
            ..fields
        }))
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
    store
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

/// An archived dependency, finished or cancelled, no longer blocks: done
/// then archived, and ready then archived, each let their dependent start.
#[test]
fn ready_counts_an_archived_dependency_as_satisfied() {
    let store = board();
    let ada = Identity {
        name: "Ada".into(),
        email: "ada@example.com".into(),
    };
    let task = |fields: TaskFields| {
        Task::new(finished(TaskFields {
            created: at("08:00:00"),
            updated: at("08:00:00"),
            ..fields
        }))
        .unwrap()
    };
    let archived = |from: TaskStatus, id: &str| {
        let before = task(TaskFields {
            status: from,
            ..fields(id)
        });
        let after = move_task(
            &before,
            TaskStatus::Archived,
            &ada,
            &[],
            &[],
            &at("09:00:00"),
        )
        .unwrap();
        assert_eq!(after.fields().status, TaskStatus::Archived);
        after
    };
    let tasks = vec![
        archived(TaskStatus::Done, "finished"),
        archived(TaskStatus::Ready, "cancelled"),
        task(TaskFields {
            depends_on: vec![slug("finished")],
            ..fields("after-finished")
        }),
        task(TaskFields {
            depends_on: vec![slug("cancelled")],
            ..fields("after-cancelled")
        }),
        task(TaskFields {
            depends_on: vec![slug("finished"), slug("cancelled")],
            ..fields("after-both")
        }),
    ];
    let head = store.read().unwrap().head;
    store.write(&head, &tasks, "seed").unwrap();
    let ready = QueryBoard::new(Arc::new(store)).ready().unwrap();
    let ids: Vec<_> = ready
        .tasks
        .iter()
        .map(|task| task.fields().id.as_str().to_owned())
        .collect();
    assert_eq!(ids, ["after-both", "after-cancelled", "after-finished"]);
}

/// Every listing names the head its tasks were read at: the board moves
/// on just after each read (another writer adds a ready task `z`), so a
/// listing that read the board twice would pair the new head with the old
/// tasks, or the old head with `z`.
#[test]
fn every_listing_names_the_head_its_tasks_were_read_at() {
    type Listing = fn(&QueryBoard) -> BoardListing;
    let listings: [Listing; 5] = [
        |query| query.ready().unwrap(),
        |query| query.takeable(&at("10:00:00")).unwrap(),
        |query| query.by_status(TaskStatus::Ready).unwrap(),
        |query| query.children(&slug("a")).unwrap(),
        |query| query.dependents(&slug("a")).unwrap(),
    ];
    for listing in listings {
        let store = seeded();
        let read_at = store.read().unwrap().head;
        store.after_next_read(|board| {
            let head = board.read().unwrap().head;
            let z = Task::new(TaskFields {
                created: at("08:00:00"),
                updated: at("08:00:00"),
                parent: Some(slug("a")),
                depends_on: vec![slug("a")],
                ..fields("z")
            })
            .unwrap();
            board.write(&head, &[z], "board: add z").unwrap();
        });
        let found = listing(&QueryBoard::new(Arc::new(store.client())));
        assert_eq!(found.head, read_at);
        assert!(
            found
                .tasks
                .iter()
                .all(|task| task.fields().id.as_str() != "z"),
            "{:?}",
            found.tasks
        );
        assert_ne!(store.read().unwrap().head, read_at, "the board moved on");
    }
}
