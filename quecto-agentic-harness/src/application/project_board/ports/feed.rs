//! The board's change feed (epic #2705 B10): which revisions the board
//! made, in order, told by number and kind alone. No event carries an
//! artifact's text, a message, an identity or a principal: a reader that
//! wants the change reads the board.
use std::sync::Arc;

use super::BoardStoreError;

/// The most revisions one batch holds: `revisions?limit=` takes 1 to this.
pub const MAX_REVISION_BATCH: usize = 500;

/// A revision the board made: its number (unique and increasing across
/// epochs) and the epoch it was made in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct BoardRevision {
    pub epoch: u64,
    pub revision: u64,
}

/// The board's newest revision and its current epoch; revision 0 while
/// there is no board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeedTip {
    pub epoch: u64,
    pub revision: u64,
}

/// A batch's size: 1 to [`MAX_REVISION_BATCH`], an allowlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchLimit(usize);

impl BatchLimit {
    pub const MOST: Self = Self(MAX_REVISION_BATCH);

    pub fn of(limit: usize) -> Option<Self> {
        (1..=MAX_REVISION_BATCH)
            .contains(&limit)
            .then_some(Self(limit))
    }

    pub fn get(self) -> usize {
        self.0
    }
}

/// The revisions after a cursor, oldest first, at most a [`BatchLimit`];
/// and the tip they were read against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionBatch {
    pub revisions: Vec<BoardRevision>,
    pub tip: FeedTip,
}

/// What a subscription holds for its reader now: the revisions published
/// since it last drained, oldest first, or that it fell behind (its
/// bounded queue overflowed, so they were dropped: its reader is reset).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedDrained {
    Revisions(Vec<BoardRevision>),
    Reset,
}

/// Called, never blocking, when a subscription may have something new.
pub type FeedWake = Arc<dyn Fn() + Send + Sync>;

/// One subscriber's place in the feed, given up when dropped.
pub trait FeedSubscription: Send {
    /// What it holds now; never blocks.
    fn drain(&mut self) -> FeedDrained;
}

/// Port: the board's change feed, as its store tells it.
pub trait BoardFeed: Send + Sync {
    /// The board's tip.
    ///
    /// # Errors
    /// The store could not be read.
    fn tip(&self) -> Result<FeedTip, BoardStoreError>;

    /// The revisions after `after`, oldest first, at most `limit`.
    ///
    /// # Errors
    /// The store could not be read.
    fn revisions(&self, after: u64, limit: BatchLimit) -> Result<RevisionBatch, BoardStoreError>;

    /// A subscription to every revision committed from now on; `wake` is
    /// called as each is published.
    fn subscribe(&self, wake: FeedWake) -> Box<dyn FeedSubscription>;
}

/// An event of the feed, an allowlist: a revision, the epoch the next
/// revisions are in, or a reset (read on from revision `after`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedEvent {
    Epoch { epoch: u64 },
    Revision(BoardRevision),
    Reset { after: u64 },
}

/// An event and the cursor a reader resumes from after it (its SSE id).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Told {
    pub id: u64,
    pub event: FeedEvent,
}

impl Told {
    /// An id as a reader sends it back (`Last-Event-ID`) or a stream tells
    /// it: 1 to 20 ASCII digits that fit a `u64`, nothing else.
    pub fn parse_id(text: &str) -> Option<u64> {
        let digits = (1..=20).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit());
        digits.then(|| text.parse().ok()).flatten()
    }
}

/// Whether a watcher goes on after an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchFlow {
    Continue,
    Stop,
}

/// Port: a client's watch of a board server's feed (`quecto board watch`).
pub trait BoardWatch {
    /// Hands `each` every event from revision `after` (none: from now),
    /// reconnecting from the last revision seen when the stream ends,
    /// until `each` says stop.
    ///
    /// # Errors
    /// The server could not be reached or refused the watch.
    fn watch(
        &self,
        after: Option<u64>,
        each: &mut dyn FnMut(FeedEvent) -> WatchFlow,
    ) -> Result<(), String>;
}
