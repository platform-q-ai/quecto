//! The board's change feed from its SQLite file (epic #2705 B10): the
//! revisions table read by number, and the notifier a served write
//! publishes its new revisions to once it commits. Each subscriber has a
//! bounded queue: the publisher never waits on a reader, and a reader that
//! falls [`QUEUE`] revisions behind is reset (its queue dropped) to resume
//! from the store.
use super::*;
use std::collections::VecDeque;
use std::sync::{PoisonError, Weak};

/// The most revisions a subscriber's queue holds before it is reset.
pub const QUEUE: usize = 256;

/// The notifier: every live subscriber's queue.
#[derive(Default)]
pub struct FeedHub {
    subscribers: Mutex<Vec<Weak<Subscriber>>>,
}

struct Subscriber {
    queue: Mutex<Queue>,
    wake: FeedWake,
}

#[derive(Default)]
struct Queue {
    revisions: VecDeque<BoardRevision>,
    reset: bool,
}

impl FeedHub {
    /// A subscriber from now on, woken by `wake`.
    pub fn subscribe(&self, wake: FeedWake) -> Box<dyn FeedSubscription> {
        let subscriber = Arc::new(Subscriber {
            queue: Mutex::default(),
            wake,
        });
        // Each change leaves the list whole, so a poisoned lock is as good.
        let mut all = self
            .subscribers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        all.retain(|held| held.strong_count() > 0);
        all.push(Arc::downgrade(&subscriber));
        Box::new(Subscription(subscriber))
    }

    /// Queues `revisions` (oldest first) for every subscriber, and wakes
    /// it; one whose queue would pass [`QUEUE`] is reset instead.
    pub fn publish(&self, revisions: &[BoardRevision]) {
        self.each(
            |queue| match queue.revisions.len() + revisions.len() <= QUEUE {
                true => queue.revisions.extend(revisions),
                false => queue.reset(),
            },
        );
    }

    /// Resets every subscriber: what was committed could not be read.
    pub fn reset_all(&self) {
        self.each(Queue::reset);
    }

    fn each(&self, mut change: impl FnMut(&mut Queue)) {
        let live: Vec<Arc<Subscriber>> = {
            let mut all = self
                .subscribers
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            all.retain(|held| held.strong_count() > 0);
            all.iter().filter_map(Weak::upgrade).collect()
        };
        for subscriber in live {
            {
                let mut queue = subscriber
                    .queue
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                match queue.reset {
                    true => {}
                    false => change(&mut queue),
                }
            }
            (subscriber.wake)();
        }
    }
}

impl Queue {
    fn reset(&mut self) {
        self.revisions = VecDeque::new();
        self.reset = true;
    }
}

/// A subscriber's handle: it leaves the hub when dropped.
struct Subscription(Arc<Subscriber>);

impl FeedSubscription for Subscription {
    fn drain(&mut self) -> FeedDrained {
        let mut queue = self.0.queue.lock().unwrap_or_else(PoisonError::into_inner);
        match queue.reset {
            true => FeedDrained::Reset,
            false => FeedDrained::Revisions(queue.revisions.drain(..).collect()),
        }
    }
}

/// The newest revision, and the newest epoch (0 and 0 with no board).
const TIP: &str = "SELECT (SELECT coalesce(max(revision),0) FROM revisions), \
                   (SELECT coalesce(max(epoch),0) FROM epochs)";

/// The tip `c` reads.
pub(super) fn tip(c: &Connection) -> Result<FeedTip, BoardStoreError> {
    let (revision, epoch): (i64, i64) = c
        .query_row(TIP, [], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(unavailable)?;
    Ok(FeedTip {
        epoch: u64::try_from(epoch).map_err(unavailable)?,
        revision: u64::try_from(revision).map_err(unavailable)?,
    })
}

/// The revisions after `after`, oldest first, at most `limit`; each one's
/// epoch read from its head (`<epoch>:<revision>`, digits alone).
pub(super) fn after(
    c: &Connection,
    after: u64,
    limit: usize,
) -> Result<Vec<BoardRevision>, BoardStoreError> {
    let (after, limit) = (
        i64::try_from(after).unwrap_or(i64::MAX),
        i64::try_from(limit).map_err(unavailable)?,
    );
    let mut rows = c
        .prepare_cached(
            "SELECT revision, head FROM revisions WHERE revision > ?1 ORDER BY revision LIMIT ?2",
        )
        .map_err(unavailable)?;
    let rows = rows
        .query_map([after, limit], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(unavailable)?;
    rows.map(|row| {
        let (revision, head) = row.map_err(unavailable)?;
        let revision = u64::try_from(revision).map_err(unavailable)?;
        let digits = |text: &str| {
            let shaped = (1..=20).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit());
            shaped.then(|| text.parse::<u64>().ok()).flatten()
        };
        match head.split_once(':').map(|(e, r)| (digits(e), digits(r))) {
            Some((Some(epoch), Some(at))) if at == revision => {
                Ok(BoardRevision { epoch, revision })
            }
            _ => Err(unavailable(format!(
                "revision {revision} has a malformed head"
            ))),
        }
    })
    .collect()
}

impl BoardFeed for SqliteBoardOps {
    fn tip(&self) -> Result<FeedTip, BoardStoreError> {
        let mut c = self.reads.connection.lock().map_err(unavailable)?;
        let tx = storage::Scope::begin(&mut c, false)?;
        tip(&tx)
    }

    fn revisions(&self, from: u64, limit: BatchLimit) -> Result<RevisionBatch, BoardStoreError> {
        let mut c = self.reads.connection.lock().map_err(unavailable)?;
        // One read transaction: the batch and its tip are of one snapshot.
        let tx = storage::Scope::begin(&mut c, false)?;
        let revisions = after(&tx, from, limit.get())?;
        Ok(RevisionBatch {
            revisions,
            tip: tip(&tx)?,
        })
    }

    fn subscribe(&self, wake: FeedWake) -> Box<dyn FeedSubscription> {
        self.feed.subscribe(wake)
    }
}

#[cfg(test)]
#[path = "feed_tests.rs"]
mod tests;
