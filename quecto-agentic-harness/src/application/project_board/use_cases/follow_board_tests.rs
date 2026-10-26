//! The follower (epic #2705 B10): each revision told exactly once, in
//! order, with no gap, over a feed whose store and queue are held here.
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use super::*;
use crate::application::project_board::ports::feed::{FeedTip, RevisionBatch};

/// The queue a subscriber of [`Feed`] holds before it is reset.
const QUEUE: usize = 4;

#[derive(Default)]
struct Queue {
    revisions: VecDeque<BoardRevision>,
    reset: bool,
}

/// A store's revisions and its subscribers' bounded queues; `reads`
/// counts the store reads, `out_of_order` makes the store lie.
#[derive(Default)]
struct Feed {
    committed: Mutex<Vec<BoardRevision>>,
    epoch: Mutex<u64>,
    queues: Mutex<Vec<Arc<Mutex<Queue>>>>,
    reads: Mutex<usize>,
    out_of_order: Mutex<bool>,
}

impl Feed {
    fn new() -> Arc<Self> {
        let feed = Self::default();
        *feed.epoch.lock().unwrap() = 1;
        Arc::new(feed)
    }

    /// Commits the next revision in the current epoch; published unless
    /// `quiet` (a commit made before a subscriber's queue saw it).
    fn commit(&self, quiet: bool) -> u64 {
        let epoch = *self.epoch.lock().unwrap();
        let mut committed = self.committed.lock().unwrap();
        let revision = BoardRevision {
            epoch,
            revision: committed.len() as u64 + 1,
        };
        committed.push(revision);
        drop(committed);
        if !quiet {
            for queue in self.queues.lock().unwrap().iter() {
                let mut queue = queue.lock().unwrap();
                match queue.revisions.len() < QUEUE {
                    true => queue.revisions.push_back(revision),
                    false => {
                        queue.revisions.clear();
                        queue.reset = true;
                    }
                }
            }
        }
        revision.revision
    }

    fn commits(&self, count: usize) {
        (0..count).for_each(|_| {
            self.commit(false);
        });
    }
}

struct Subscribed(Arc<Mutex<Queue>>);

impl FeedSubscription for Subscribed {
    fn drain(&mut self) -> FeedDrained {
        let mut queue = self.0.lock().unwrap();
        match queue.reset {
            true => FeedDrained::Reset,
            false => FeedDrained::Revisions(queue.revisions.drain(..).collect()),
        }
    }
}

impl BoardFeed for Feed {
    fn tip(&self) -> Result<FeedTip, BoardStoreError> {
        Ok(FeedTip {
            epoch: *self.epoch.lock().unwrap(),
            revision: self.committed.lock().unwrap().len() as u64,
        })
    }

    fn revisions(&self, after: u64, limit: BatchLimit) -> Result<RevisionBatch, BoardStoreError> {
        *self.reads.lock().unwrap() += 1;
        let tip = self.tip()?;
        let committed = self.committed.lock().unwrap();
        let mut revisions: Vec<_> = committed
            .iter()
            .filter(|revision| revision.revision > after)
            .take(limit.get())
            .copied()
            .collect();
        if *self.out_of_order.lock().unwrap() {
            revisions.reverse();
        }
        Ok(RevisionBatch { revisions, tip })
    }

    fn subscribe(&self, _wake: FeedWake) -> Box<dyn FeedSubscription> {
        let queue = Arc::new(Mutex::new(Queue::default()));
        self.queues.lock().unwrap().push(Arc::clone(&queue));
        Box::new(Subscribed(queue))
    }
}

fn following(feed: &Arc<Feed>, after: Option<u64>) -> Follower {
    FollowBoard::new(Arc::clone(feed) as Arc<dyn BoardFeed>).follow(after, Arc::new(|| {}))
}

/// Every event a follower tells until it has nothing more, and whether it
/// ended.
fn drained(follower: &mut Follower) -> (Vec<Told>, bool) {
    let mut all = Vec::new();
    loop {
        match follower.step().unwrap() {
            FollowStep::Events { told, more } => {
                all.extend(told);
                if !more {
                    return (all, false);
                }
            }
            FollowStep::Ended(told) => {
                all.extend(told);
                return (all, true);
            }
        }
    }
}

/// The revision numbers told, in order.
fn numbers(told: &[Told]) -> Vec<u64> {
    told.iter()
        .filter_map(|told| match told.event {
            FeedEvent::Revision(revision) => {
                assert_eq!(told.id, revision.revision, "a revision's id is itself");
                Some(revision.revision)
            }
            FeedEvent::Epoch { .. } | FeedEvent::Reset { .. } => None,
        })
        .collect()
}

#[test]
fn a_follow_from_now_tells_the_epoch_then_each_new_revision_once() {
    let feed = Feed::new();
    feed.commits(3);
    let mut follower = following(&feed, None);
    let (first, ended) = drained(&mut follower);
    assert!(!ended);
    assert_eq!(
        first,
        vec![Told {
            id: 3,
            event: FeedEvent::Epoch { epoch: 1 }
        }],
        "from now: the epoch, resumable from the tip"
    );
    feed.commits(2);
    let (told, _) = drained(&mut follower);
    assert_eq!(numbers(&told), vec![4, 5]);
    assert_eq!(numbers(&drained(&mut follower).0), Vec::<u64>::new());
}

/// Red test (plan B10): a resume after a cut connection is gap-free,
/// each revision exactly once, however the commits fall around the cut.
#[test]
fn a_resume_after_a_cut_is_gap_free_with_each_revision_once() {
    let feed = Feed::new();
    feed.commits(5);
    let mut first = following(&feed, Some(0));
    let (told, _) = drained(&mut first);
    let mut seen = numbers(&told);
    assert_eq!(seen, vec![1, 2, 3, 4, 5]);
    feed.commits(2);
    let (told, _) = drained(&mut first);
    seen.extend(numbers(&told));
    let last = told.last().unwrap().id;
    // The connection is cut; revisions land while no one listens.
    drop(first);
    feed.commits(4);
    let mut resumed = following(&feed, Some(last));
    // One lands between the resume's subscribe and its store read: it is
    // both queued and in the store, and is told once.
    feed.commit(false);
    let (told, _) = drained(&mut resumed);
    seen.extend(numbers(&told));
    feed.commits(3);
    seen.extend(numbers(&drained(&mut resumed).0));
    assert_eq!(
        seen,
        (1..=15).collect::<Vec<_>>(),
        "every revision, once, in order"
    );
}

/// A revision the queue missed (published before it subscribed, committed
/// after the store read) is read from the store: the queue's gap is filled.
#[test]
fn a_gap_in_the_queue_is_filled_from_the_store() {
    let feed = Feed::new();
    let mut follower = following(&feed, Some(0));
    assert_eq!(numbers(&drained(&mut follower).0), Vec::<u64>::new());
    feed.commit(true);
    feed.commit(false);
    let (told, _) = drained(&mut follower);
    assert_eq!(numbers(&told), vec![1, 2]);
}

/// Red test (plan B10): a stalled subscriber, whose queue overflowed,
/// is reset: told to resume from its last revision, and ended.
#[test]
fn a_stalled_subscriber_is_reset_and_resumes_gap_free() {
    let feed = Feed::new();
    feed.commits(2);
    let mut follower = following(&feed, Some(0));
    assert_eq!(numbers(&drained(&mut follower).0), vec![1, 2]);
    // It reads nothing while more than its queue holds is published.
    feed.commits(QUEUE + 1);
    let (told, ended) = drained(&mut follower);
    assert!(ended, "a reset ends the follow");
    assert_eq!(
        told,
        vec![Told {
            id: 2,
            event: FeedEvent::Reset { after: 2 }
        }]
    );
    let mut resumed = following(&feed, Some(2));
    let expected: Vec<u64> = (3..=(QUEUE as u64 + 3)).collect();
    assert_eq!(numbers(&drained(&mut resumed).0), expected);
}

#[test]
fn a_cursor_past_the_tip_is_reset_to_the_tip() {
    let feed = Feed::new();
    feed.commits(3);
    let mut follower = following(&feed, Some(9));
    let (told, ended) = drained(&mut follower);
    assert!(ended);
    assert_eq!(
        told,
        vec![Told {
            id: 3,
            event: FeedEvent::Reset { after: 3 }
        }]
    );
}

/// A reader far behind is caught up a batch a step, each step asking to
/// be stepped again at once.
#[test]
fn a_reader_far_behind_is_caught_up_in_batches() {
    let feed = Feed::new();
    feed.commits(1_100);
    let mut follower = following(&feed, Some(0));
    let mut steps = Vec::new();
    loop {
        match follower.step().unwrap() {
            FollowStep::Events { told, more } => {
                steps.push(numbers(&told).len());
                if !more {
                    break;
                }
            }
            FollowStep::Ended(_) => panic!("never reset"),
        }
    }
    assert_eq!(steps, vec![500, 500, 100]);
    assert_eq!(*feed.reads.lock().unwrap(), 3);
}

#[test]
fn a_new_epoch_is_told_before_its_first_revision() {
    let feed = Feed::new();
    feed.commits(1);
    let mut follower = following(&feed, Some(0));
    drained(&mut follower);
    *feed.epoch.lock().unwrap() = 2;
    feed.commit(false);
    let (told, _) = drained(&mut follower);
    assert_eq!(
        told,
        vec![
            Told {
                id: 1,
                event: FeedEvent::Epoch { epoch: 2 }
            },
            Told {
                id: 2,
                event: FeedEvent::Revision(BoardRevision {
                    epoch: 2,
                    revision: 2
                })
            },
        ]
    );
}

#[test]
fn revisions_out_of_order_are_a_failure_never_told() {
    let feed = Feed::new();
    feed.commits(3);
    *feed.out_of_order.lock().unwrap() = true;
    let mut follower = following(&feed, Some(0));
    assert!(matches!(follower.step(), Err(ServeError::Board(_))));
}
