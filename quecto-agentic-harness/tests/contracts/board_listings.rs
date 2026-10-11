//! Contract for [`BoardListings`] (#2476): a listing means what
//! `QueryBoard`'s listing of the same name means over the same store,
//! carries the head it was read at and every file set aside, and follows
//! the board as it moves. Here it runs against the SQLite index over a git
//! store (two clones of one bare remote).
use super::project_board_git_store::Remote;
use super::project_board_store::task;
use quecto::application::project_board::dto::BoardListing;
use quecto::application::project_board::ports::{
    BoardListings, BoardStoreError, BoardWrite, ProjectBoardStore,
};
use quecto::application::project_board::use_cases::QueryBoard;
use quecto::domain::project_board::entities::claim::{Claim, Identity};
use quecto::domain::project_board::entities::task::Task;
use quecto::domain::project_board::value_objects::slug::Slug;
use quecto::domain::project_board::value_objects::timestamp::Timestamp;
use quecto::domain::project_board::value_objects::vocabulary::TaskStatus;
use quecto::infrastructure::persistence::project_board::index::BoardIndex;
use quecto::infrastructure::persistence::project_board::indexed_listings::IndexedBoardListings;
use quecto::infrastructure::persistence::project_board::store::GitBoardStore;
use std::sync::Arc;

fn at(time: &str) -> Timestamp {
    Timestamp::parse(&format!("2026-10-10T{time}Z")).unwrap()
}

/// `id` in `status`, under `parent`, after `depends_on`.
fn shaped(id: &str, status: TaskStatus, parent: Option<&str>, depends_on: &[&str]) -> Task {
    let mut fields = task(id, id).into_fields();
    fields.status = status;
    fields.parent = parent.map(|parent| Slug::parse(parent).unwrap());
    fields.depends_on = depends_on
        .iter()
        .map(|id| Slug::parse(id).unwrap())
        .collect();
    Task::new(fields).unwrap()
}

/// `id` in `status`, held since 09:00 until `expires` (or not held).
fn held(id: &str, status: TaskStatus, expires: Option<&str>) -> Task {
    let mut fields = task(id, id).into_fields();
    fields.status = status;
    fields.updated = at("09:00:00");
    fields.claim = expires.map(|expires| Claim {
        holder: Identity {
            name: "Ada".into(),
            email: "ada@example.com".into(),
        },
        since: at("09:00:00"),
        expires: at(expires),
    });
    Task::new(fields).unwrap()
}

/// Every case the listings tell apart; `b` is the task `spoil` breaks.
fn cases() -> Vec<Task> {
    vec![
        shaped("epic", TaskStatus::Draft, None, &[]),
        shaped("b", TaskStatus::Ready, Some("epic"), &[]),
        shaped("r-alone", TaskStatus::Ready, Some("epic"), &[]),
        shaped("r-after-done", TaskStatus::Ready, None, &["done"]),
        shaped("r-after-archived", TaskStatus::Ready, None, &["archived"]),
        shaped("r-after-missing", TaskStatus::Ready, None, &["missing"]),
        shaped("r-after-ready", TaskStatus::Ready, None, &["r-alone"]),
        shaped("r-after-b", TaskStatus::Ready, None, &["b"]),
        shaped("done", TaskStatus::Done, Some("epic"), &["b"]),
        shaped("archived", TaskStatus::Archived, None, &[]),
        held("c-expired", TaskStatus::Claimed, Some("10:00:00")),
        held("w-live", TaskStatus::InProgress, Some("11:00:00")),
        held("v-expired", TaskStatus::Review, Some("10:00:00")),
        held("v-unheld", TaskStatus::Review, None),
        held("b-expired", TaskStatus::Blocked, Some("10:00:00")),
    ]
}

fn same(left: Result<BoardListing, BoardStoreError>, right: BoardListing, what: &str) {
    assert_eq!(left, Ok(right), "{what}");
}

/// [`same`], for a listing the cases make non-empty, so the parity is
/// never two empty lists agreeing.
fn same_and_some(left: Result<BoardListing, BoardStoreError>, right: BoardListing, what: &str) {
    assert!(!right.tasks.is_empty(), "{what} lists nothing");
    same(left, right, what);
}

/// Before the board exists every listing answers `NoBoard`; after, each
/// listing equals `QueryBoard`'s over the same store, a file set aside
/// (`spoil` breaks `tasks/b.json`) included, and follows a write.
pub fn check_listings_mean_query_board(
    listings: &dyn BoardListings,
    store: Arc<dyn ProjectBoardStore>,
    spoil: impl FnOnce(),
) {
    assert_eq!(listings.ready(), Err(BoardStoreError::NoBoard));
    store.bootstrap().unwrap();
    let head = store.read().unwrap().head;
    let seeded = store.write(&head, &cases(), "board: seed");
    assert!(matches!(seeded, Ok(BoardWrite::Written(_))), "{seeded:?}");
    spoil();
    let query = QueryBoard::new(Arc::clone(&store));
    same_and_some(listings.ready(), query.ready().unwrap(), "ready");
    same_and_some(
        listings.takeable(&at("10:30:00")),
        query.takeable(&at("10:30:00")).unwrap(),
        "takeable at 10:30",
    );
    same_and_some(
        listings.by_status(TaskStatus::Ready),
        query.by_status(TaskStatus::Ready).unwrap(),
        "ready by status",
    );
    for (listing, of) in [("children", "epic"), ("dependents", "b")] {
        let id = Slug::parse(of).unwrap();
        let (left, right) = match listing {
            "children" => (listings.children(&id), query.children(&id).unwrap()),
            _ => (listings.dependents(&id), query.dependents(&id).unwrap()),
        };
        same_and_some(left, right, &format!("{listing} of {of}"));
    }
    for time in ["09:59:59", "10:00:00", "10:30:00", "11:00:00"] {
        same(
            listings.takeable(&at(time)),
            query.takeable(&at(time)).unwrap(),
            time,
        );
    }
    for status in TaskStatus::ALL.iter().copied() {
        same(
            listings.by_status(status),
            query.by_status(status).unwrap(),
            status.as_str(),
        );
    }
    for id in ["epic", "r-alone", "b", "missing"] {
        let id = Slug::parse(id).unwrap();
        same(
            listings.children(&id),
            query.children(&id).unwrap(),
            id.as_str(),
        );
        same(
            listings.dependents(&id),
            query.dependents(&id).unwrap(),
            id.as_str(),
        );
    }
    assert!(
        !listings.ready().unwrap().quarantined.is_empty(),
        "the spoilt file is listed"
    );
    let head = store.read().unwrap().head;
    let moved = store.write(
        &head,
        &[shaped("late", TaskStatus::Ready, None, &[])],
        "board: add late",
    );
    assert!(matches!(moved, Ok(BoardWrite::Written(_))), "{moved:?}");
    same(
        listings.ready(),
        query.ready().unwrap(),
        "ready after the board moved",
    );
}

#[test]
fn the_sqlite_index_lists_what_query_board_lists() {
    let remote = Remote::new();
    let store: Arc<GitBoardStore> = Arc::from(remote.client(0, "Alice"));
    let index = BoardIndex::of(&store).unwrap();
    let listings = IndexedBoardListings::new(index, store.clone(), store.clone());
    check_listings_mean_query_board(&listings, store, || remote.spoil_b());
}
