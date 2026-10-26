//! A listing's summaries are opt-in and never cost the listing: asked for
//! none, a listing reads only what it lists. Asked for them, they are
//! read from the board the listing was read at; while the board moves
//! between the listing and that read, the listing is read again, at most
//! [`MAX_LIST_ATTEMPTS`] times, and past that, or when the read fails,
//! the listing is answered without them, saying why.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::*;
use crate::application::project_board::dto::{NewBoardTask, NewBoardTaskStatus};
use crate::application::project_board::operations::NoPullRequestHost;
use crate::application::project_board::operations::query_listings::assembled;
use crate::application::project_board::ports::{
    BoardArtifact, BoardBootstrap, BoardHead, BoardListings, BoardSnapshot, BoardWrite,
    ProjectBoardStore, SetAsideClear,
};
use crate::application::project_board::task_summary::{RowSummaries, SkipReason};
use crate::application::project_board::use_cases::QueryBoard;
use crate::application::project_board::use_cases::fixtures_tests::{SetClock, new_task};
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;

/// The board the operations read whole, counting each read; once
/// `failing`, a read fails as an unreachable store's does.
struct Counted {
    inner: MemoryBoardStore,
    reads: AtomicUsize,
    failing: AtomicBool,
}

impl ProjectBoardStore for Counted {
    fn artifact_at(
        &self,
        rev: &crate::application::project_board::ports::BoardHead,
        key: &crate::application::project_board::ports::ArtifactKey,
    ) -> Result<
        Option<crate::application::project_board::ports::BoardArtifact>,
        crate::application::project_board::ports::BoardStoreError,
    > {
        self.inner.artifact_at(rev, key)
    }

    fn read(&self) -> Result<BoardSnapshot, BoardStoreError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        match self.failing.load(Ordering::SeqCst) {
            true => Err(BoardStoreError::Unavailable("the remote is gone".into())),
            false => self.inner.read(),
        }
    }

    fn write(
        &self,
        expected: &BoardHead,
        artifacts: &[BoardArtifact],
        message: &str,
    ) -> Result<BoardWrite, BoardStoreError> {
        self.inner.write(expected, artifacts, message)
    }

    fn clear_set_aside(
        &self,
        expected: &BoardHead,
        clear: &SetAsideClear,
        message: &str,
    ) -> Result<BoardWrite, BoardStoreError> {
        self.inner.clear_set_aside(expected, clear, message)
    }

    fn bootstrap(&self) -> Result<BoardBootstrap, BoardStoreError> {
        self.inner.bootstrap()
    }

    fn writer(&self) -> Identity {
        self.inner.writer()
    }
}

/// Listings over `QueryBoard` whose first `stale` answers name a head the
/// board is not at, as an index read before the board moved does; past
/// `fail_after` calls, a listing fails; once `forged`, a listing at the
/// board's head lists its tasks other than the board holds them, as a
/// broken or forged index does.
struct Moving {
    query: QueryBoard,
    stale: usize,
    fail_after: usize,
    calls: AtomicUsize,
    forged: AtomicBool,
}

impl Moving {
    fn all_listed(&self) -> Result<BoardListing, BoardStoreError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call >= self.fail_after {
            return Err(BoardStoreError::Unavailable("the index is gone".into()));
        }
        let listing = self.query.all().map_err(|error| match error {
            ProjectBoardError::Store(error) => error,
            other => BoardStoreError::Unavailable(other.to_string()),
        })?;
        let mut listing = match call < self.stale {
            true => BoardListing {
                head: BoardHead("an older head".into()),
                ..listing
            },
            false => listing,
        };
        if self.forged.load(Ordering::SeqCst) {
            listing.tasks = listing
                .tasks
                .into_iter()
                .map(|task| {
                    let fields = task.into_fields();
                    Task::new(TaskFields {
                        title: "Not the board's".into(),
                        ..fields
                    })
                    .unwrap()
                })
                .collect();
        }
        Ok(listing)
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

struct Board {
    ops: RunBoardOperation,
    listings: Arc<Moving>,
    store: Arc<Counted>,
}

/// The operations over a board of one ready task, `a`, listed by
/// [`Moving`] answering `stale` stale listings first and failing past
/// `fail_after` listings.
fn moving(stale: usize, fail_after: usize) -> Board {
    let inner = MemoryBoardStore::default();
    inner.bootstrap().unwrap();
    let listings = Arc::new(Moving {
        query: QueryBoard::new(Arc::new(inner.client())),
        stale,
        fail_after,
        calls: AtomicUsize::new(0),
        forged: AtomicBool::new(false),
    });
    let store = Arc::new(Counted {
        inner: inner.client(),
        reads: AtomicUsize::new(0),
        failing: AtomicBool::new(false),
    });
    let ops = assembled(
        Arc::clone(&store) as Arc<dyn ProjectBoardStore>,
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
    store.reads.store(0, Ordering::SeqCst);
    Board {
        ops,
        listings,
        store,
    }
}

impl Board {
    fn listed(&self, request: &BoardRequest, summaries: ListSummaries) -> Listed {
        match self.ops.execute_noted_with(request, None, summaries) {
            Ok(BoardOutcome::Listing(listed)) => listed,
            other => panic!("{request:?}: {other:?}"),
        }
    }

    fn listings(&self) -> usize {
        self.listings.calls.load(Ordering::SeqCst)
    }

    fn reads(&self) -> usize {
        self.store.reads.load(Ordering::SeqCst)
    }
}

fn every_listing() -> [BoardRequest; 3] {
    [
        BoardRequest::List(ListFilter::All),
        BoardRequest::ListReady,
        BoardRequest::ListTakeable,
    ]
}

/// Asked for none, a listing is listed once and the board is never read
/// whole, however it moves: exactly as before summaries.
#[test]
fn a_listing_without_summaries_reads_only_what_it_lists() {
    for request in every_listing() {
        let board = moving(usize::MAX, usize::MAX);
        let listed = board.listed(&request, ListSummaries::Omitted);
        assert_eq!(listed.summaries(), &RowSummaries::Omitted, "{request:?}");
        assert!(listed.entries().all(|(_, summary)| summary.is_none()));
        assert_eq!((board.listings(), board.reads()), (1, 0), "{request:?}");
        // `execute` is a listing without summaries.
        let plain = board.ops.execute(&request).unwrap();
        assert_eq!(plain, BoardOutcome::Listing(listed), "{request:?}");
        assert_eq!((board.listings(), board.reads()), (2, 0), "{request:?}");
    }
}

#[test]
fn a_listing_with_summaries_summarises_each_task() {
    for request in every_listing() {
        let board = moving(0, usize::MAX);
        let listed = board.listed(&request, ListSummaries::Wanted);
        let RowSummaries::Given(summaries) = listed.summaries() else {
            panic!("{listed:?}")
        };
        assert_eq!(summaries.len(), 1, "{request:?}");
        assert!(listed.entries().all(|(_, summary)| summary.is_some()));
        assert_eq!((board.listings(), board.reads()), (1, 1), "{request:?}");
    }
}

#[test]
fn a_listing_read_before_the_board_moved_is_read_again() {
    for request in every_listing() {
        let board = moving(MAX_LIST_ATTEMPTS - 1, usize::MAX);
        let listed = board.listed(&request, ListSummaries::Wanted);
        assert!(
            matches!(listed.summaries(), RowSummaries::Given(_)),
            "{listed:?}"
        );
        assert_eq!(board.listings(), MAX_LIST_ATTEMPTS, "{request:?}");
        assert_eq!(board.reads(), MAX_LIST_ATTEMPTS, "{request:?}");
    }
}

/// A board that keeps moving (a swarm writing as it runs) never refuses
/// the listing: it is answered without summaries, as contended.
#[test]
fn a_board_that_keeps_moving_skips_the_summaries_as_contended() {
    for request in every_listing() {
        let board = moving(usize::MAX, usize::MAX);
        let listed = board.listed(&request, ListSummaries::Wanted);
        assert_eq!(
            listed.summaries(),
            &RowSummaries::Skipped(SkipReason::Contended),
            "{request:?}"
        );
        assert_eq!(listed.listing().tasks.len(), 1, "the listing is answered");
        assert!(listed.entries().all(|(_, summary)| summary.is_none()));
        assert_eq!(board.listings(), MAX_LIST_ATTEMPTS, "{request:?}");
        assert_eq!(board.reads(), MAX_LIST_ATTEMPTS, "{request:?}");
    }
    assert_eq!(MAX_LIST_ATTEMPTS, 3);
}

/// A listing at the board's head whose rows are not the board's (the
/// index disagrees with it) is answered at once without summaries:
/// reading it again would list the same.
#[test]
fn an_index_that_disagrees_skips_the_summaries_at_once() {
    for request in every_listing() {
        let board = moving(0, usize::MAX);
        board.listings.forged.store(true, Ordering::SeqCst);
        let listed = board.listed(&request, ListSummaries::Wanted);
        assert_eq!(listed.listing().tasks.len(), 1, "the listing is answered");
        assert_eq!(
            listed.summaries(),
            &RowSummaries::Skipped(SkipReason::IndexDisagrees),
            "{request:?}"
        );
        assert_eq!((board.listings(), board.reads()), (1, 1), "{request:?}");
    }
}

/// A whole-board read that fails skips the summaries, saying why; the
/// listing is answered.
#[test]
fn a_failed_board_read_skips_the_summaries() {
    for request in every_listing() {
        let board = moving(0, usize::MAX);
        board.store.failing.store(true, Ordering::SeqCst);
        let listed = board.listed(&request, ListSummaries::Wanted);
        assert_eq!(
            listed.summaries(),
            &RowSummaries::Skipped(SkipReason::Failed(Box::new(ProjectBoardError::Store(
                BoardStoreError::Unavailable("the remote is gone".into())
            )))),
            "{request:?}"
        );
        assert_eq!(listed.listing().tasks.len(), 1, "the listing is answered");
        assert_eq!(board.listings(), 1, "a failed read is not retried");
    }
}

/// A listing read again after the board moved that fails leaves the one
/// read before, its summaries skipped as contended: the board moved.
#[test]
fn a_listing_that_fails_when_read_again_answers_the_one_before() {
    let board = moving(usize::MAX, 1);
    let listed = board.listed(&BoardRequest::List(ListFilter::All), ListSummaries::Wanted);
    assert_eq!(
        listed.summaries(),
        &RowSummaries::Skipped(SkipReason::Contended)
    );
    assert_eq!(listed.head(), &BoardHead("an older head".into()));
    assert_eq!(board.listings(), 2);
    // The first listing failing is the listing's refusal, as ever.
    let board = moving(0, 0);
    assert!(matches!(
        board
            .ops
            .execute_noted_with(&BoardRequest::ListReady, None, ListSummaries::Wanted),
        Err(ProjectBoardError::Store(BoardStoreError::Unavailable(_)))
    ));
}

/// Summaries asked of a request that lists nothing are refused as
/// invalid.
#[test]
fn summaries_are_asked_only_of_a_listing() {
    let board = moving(0, usize::MAX);
    let show = BoardRequest::Show(
        Slug::parse("a").unwrap(),
        crate::application::project_board::operations::PlanView::default(),
    );
    assert_eq!(
        board
            .ops
            .execute_noted_with(&show, None, ListSummaries::Wanted),
        Err(ProjectBoardError::Invalid(
            crate::domain::project_board::value_objects::schema_error::SchemaError::new(
                "summary",
                crate::application::project_board::dto::SUMMARIES_LISTINGS_ONLY,
            )
        ))
    );
}
