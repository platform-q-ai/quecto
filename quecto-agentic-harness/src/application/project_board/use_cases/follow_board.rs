//! A board server's change feed (epic #2705 B10): the revisions after a
//! cursor in batches, and a follower that tells each revision exactly
//! once, in order, with no gap. A follower subscribes before it reads the
//! store, so a revision committed while it catches up is in its queue;
//! what it already told is skipped by number. A panic aborts the server
//! (ADR-0029), so every failure is a [`ServeError`].
use std::sync::Arc;

use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::feed::{
    BatchLimit, BoardFeed, BoardRevision, FeedDrained, FeedEvent, FeedSubscription, FeedWake,
    RevisionBatch, Told,
};
use crate::application::project_board::ports::{BoardStoreError, ServeError};

/// The board's change feed, as a server serves it.
pub struct FollowBoard {
    feed: Arc<dyn BoardFeed>,
}

/// What a follower has now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FollowStep {
    /// Events to send; `more`: it is still behind the store, so step again
    /// at once rather than waiting to be woken.
    Events { told: Vec<Told>, more: bool },
    /// Last events (a reset): the follow ends, and its reader resumes from
    /// the reset's `after`.
    Ended(Vec<Told>),
}

impl FollowBoard {
    pub fn new(feed: Arc<dyn BoardFeed>) -> Self {
        Self { feed }
    }

    /// The revisions after `after`, at most `limit`, and the tip.
    ///
    /// # Errors
    /// The store could not be read.
    pub fn revisions(&self, after: u64, limit: BatchLimit) -> Result<RevisionBatch, ServeError> {
        self.feed.revisions(after, limit).map_err(failed)
    }

    /// A follower from revision `after` (none: from the tip), subscribed
    /// now; `wake` is called whenever it may have more.
    pub fn follow(&self, after: Option<u64>, wake: FeedWake) -> Follower {
        Follower {
            subscription: self.feed.subscribe(wake),
            feed: Arc::clone(&self.feed),
            from: after,
            cursor: None,
            epoch: None,
            behind: true,
            ended: false,
        }
    }
}

/// One reader's follow of the feed.
pub struct Follower {
    feed: Arc<dyn BoardFeed>,
    subscription: Box<dyn FeedSubscription>,
    from: Option<u64>,
    /// The last revision told; none before the first step.
    cursor: Option<u64>,
    /// The epoch last told.
    epoch: Option<u64>,
    behind: bool,
    ended: bool,
}

impl Follower {
    /// What it has now: the store read while it is behind (at most one
    /// batch a step), then its queue. A reset queue, or a cursor past the
    /// board's tip (a reader of another board, or one restored), ends it.
    ///
    /// # Errors
    /// The store could not be read, or gave revisions out of order.
    pub fn step(&mut self) -> Result<FollowStep, ServeError> {
        assert!(!self.ended, "an ended follower is never stepped again");
        if !self.ended {
            return Ok(FollowStep::Events {
                told: vec![],
                more: false,
            });
        }
        let mut told = Vec::new();
        let mut cursor = match self.cursor {
            Some(cursor) => cursor,
            None => {
                let tip = self.feed.tip().map_err(failed)?;
                let start = self.from.unwrap_or(tip.revision);
                if start > tip.revision {
                    return Ok(self.end(told, tip.revision));
                }
                if start == tip.revision {
                    told.push(Told {
                        id: start,
                        event: FeedEvent::Epoch { epoch: tip.epoch },
                    });
                    self.epoch = Some(tip.epoch);
                }
                self.cursor = Some(start);
                start
            }
        };
        let mut queued = match self.subscription.drain() {
            FeedDrained::Revisions(queued) => queued,
            FeedDrained::Reset => return Ok(self.end(told, cursor)),
        };
        queued.retain(|queued| queued.revision > cursor);
        let gap = queued
            .first()
            .is_some_and(|first| Some(first.revision) != cursor.checked_add(1));
        if self.behind || gap {
            let batch = self
                .feed
                .revisions(cursor, BatchLimit::MOST)
                .map_err(failed)?;
            for revision in &batch.revisions {
                cursor = self.tell(&mut told, cursor, *revision)?;
            }
            self.behind =
                batch.revisions.len() == BatchLimit::MOST.get() && cursor < batch.tip.revision;
            queued.retain(|queued| queued.revision > cursor);
        }
        if !self.behind {
            for revision in queued {
                match Some(revision.revision) == cursor.checked_add(1) {
                    true => cursor = self.tell(&mut told, cursor, revision)?,
                    // A gap in the queue: the store fills it next step.
                    false => {
                        self.behind = true;
                        break;
                    }
                }
            }
        }
        self.cursor = Some(cursor);
        Ok(FollowStep::Events {
            told,
            more: self.behind,
        })
    }

    /// Tells `revision`, after its epoch when that changed: the new cursor.
    fn tell(
        &mut self,
        told: &mut Vec<Told>,
        cursor: u64,
        revision: BoardRevision,
    ) -> Result<u64, ServeError> {
        if revision.revision <= cursor {
            return Err(failed(BoardStoreError::Unavailable(
                "the board's revisions came out of order".into(),
            )));
        }
        if self.epoch != Some(revision.epoch) {
            told.push(Told {
                id: cursor,
                event: FeedEvent::Epoch {
                    epoch: revision.epoch,
                },
            });
            self.epoch = Some(revision.epoch);
        }
        told.push(Told {
            id: revision.revision,
            event: FeedEvent::Revision(revision),
        });
        Ok(revision.revision)
    }

    /// Ends the follow with a reset: read on from `after`.
    fn end(&mut self, mut told: Vec<Told>, after: u64) -> FollowStep {
        self.ended = true;
        told.push(Told {
            id: after,
            event: FeedEvent::Reset { after },
        });
        FollowStep::Ended(told)
    }
}

fn failed(error: BoardStoreError) -> ServeError {
    ServeError::Board(ProjectBoardError::Store(error))
}

#[cfg(test)]
#[path = "follow_board_tests.rs"]
mod tests;
