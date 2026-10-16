//! A listing's summaries are read from the board the listing was read at:
//! while the board moves between the listing and that read, the listing
//! is read again, at most [`MAX_LIST_ATTEMPTS`] times, then refused as
//! contended.
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::application::project_board::dto::{NewBoardTask, NewBoardTaskStatus};
use crate::application::project_board::operations::NoPullRequestHost;
use crate::application::project_board::operations::query_listings::assembled;
use crate::application::project_board::ports::{BoardHead, BoardListings, ProjectBoardStore};
use crate::application::project_board::use_cases::QueryBoard;
use crate::application::project_board::use_cases::fixtures_tests::{SetClock, new_task};
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;

/// Listings over `QueryBoard` whose first `stale` answers name a head the
/// board is not at, as an index read before the board moved does.
struct Moving {
    query: QueryBoard,
    stale: usize,
    calls: AtomicUsize,
}

impl Moving {
    fn listed(&self, listing: BoardListing) -> Result<BoardListing, BoardStoreError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(match call < self.stale {
            true => BoardListing {
                head: BoardHead("an older head".into()),
                ..listing
            },
            false => listing,
        })
    }

    fn all_listed(&self) -> Result<BoardListing, BoardStoreError> {
        let listing = self.query.all().map_err(|error| match error {
            ProjectBoardError::Store(error) => error,
            other => BoardStoreError::Unavailable(other.to_string()),
        })?;
        self.listed(listing)
    }
}

impl BoardListings for Moving {
    fn ready(&self) -> Result<BoardListing, BoardStoreError> {
        self.all_listed()
    }

    fn takeable(&self, _: &Timestamp) -> Result<BoardListing, BoardStoreError> {
        self.all_listed()
    }

    fn all(&self) -> Result<BoardListing, BoardStoreError> {
        self.all_listed()
    }

    fn by_status(&self, _: TaskStatus) -> Result<BoardListing, BoardStoreError> {
        self.all_listed()
    }

    fn children(&self, _: &Slug) -> Result<BoardListing, BoardStoreError> {
        self.all_listed()
    }

    fn dependents(&self, _: &Slug) -> Result<BoardListing, BoardStoreError> {
        self.all_listed()
    }
}

/// The operations over a board of one ready task, `a`, listed by
/// [`Moving`] answering `stale` stale listings first.
fn moving(stale: usize) -> (RunBoardOperation, Arc<Moving>) {
    let store = MemoryBoardStore::default();
    store.bootstrap().unwrap();
    let listings = Arc::new(Moving {
        query: QueryBoard::new(Arc::new(store.client())),
        stale,
        calls: AtomicUsize::new(0),
    });
    let ops = assembled(
        Arc::new(store.client()),
        SetClock::at("10:00:00"),
        Arc::clone(&listings) as Arc<dyn BoardListings>,
        Err(NoPullRequestHost::NotAPullRequestHost),
    )
    .unwrap();
    let created = ops.execute(&BoardRequest::Create(NewBoardTask {
        status: NewBoardTaskStatus::Ready,
        ..new_task("a")
    }));
    assert!(created.is_ok(), "{created:?}");
    (ops, listings)
}

fn every_listing() -> [BoardRequest; 3] {
    [
        BoardRequest::List(ListFilter::All),
        BoardRequest::ListReady,
        BoardRequest::ListTakeable,
    ]
}

#[test]
fn a_listing_read_before_the_board_moved_is_read_again() {
    for request in every_listing() {
        let (ops, listings) = moving(MAX_LIST_ATTEMPTS - 1);
        let listed = ops.execute(&request).unwrap();
        let BoardOutcome::Listing(listed) = listed else {
            panic!("{listed:?}")
        };
        assert_eq!(listed.entries().count(), 1, "{request:?}");
        assert_eq!(listings.calls.load(Ordering::SeqCst), MAX_LIST_ATTEMPTS);
    }
}

#[test]
fn a_board_that_keeps_moving_is_refused_as_contended() {
    for request in every_listing() {
        let (ops, listings) = moving(usize::MAX);
        assert_eq!(
            ops.execute(&request),
            Err(ProjectBoardError::Contended {
                attempts: MAX_LIST_ATTEMPTS
            }),
            "{request:?}"
        );
        assert_eq!(listings.calls.load(Ordering::SeqCst), MAX_LIST_ATTEMPTS);
    }
    assert_eq!(MAX_LIST_ATTEMPTS, 3);
}

/// A listing at the board's head is read once.
#[test]
fn a_listing_of_the_board_as_it_stands_is_read_once() {
    let (ops, listings) = moving(0);
    assert!(ops.execute(&BoardRequest::List(ListFilter::All)).is_ok());
    assert_eq!(listings.calls.load(Ordering::SeqCst), 1);
}
