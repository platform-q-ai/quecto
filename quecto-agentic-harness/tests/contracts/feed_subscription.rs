//! Contract for [`FeedSubscription`] (epic #2705 B10), on the SQLite
//! notifier's: a drain never blocks and hands each revision once, oldest
//! first; a subscription past its bounded queue is reset, for good, while
//! the writer goes on unhindered.
use quecto::application::project_board::ports::feed::{BoardFeed, FeedDrained, FeedSubscription};

use super::board_feed::{board, counted, revision, wrote};

fn drained(subscription: &mut Box<dyn FeedSubscription>) -> FeedDrained {
    subscription.drain()
}

#[test]
fn a_drain_hands_each_revision_once_and_never_blocks() {
    let (_dir, ops) = board();
    let (_, wake) = counted();
    let mut subscription = ops.subscribe(wake);
    assert_eq!(drained(&mut subscription), FeedDrained::Revisions(vec![]));
    wrote(&ops, 1, true, 1);
    assert_eq!(
        drained(&mut subscription),
        FeedDrained::Revisions(vec![revision(1), revision(2)])
    );
    assert_eq!(drained(&mut subscription), FeedDrained::Revisions(vec![]));
}

/// The queue holds 256: one write of 300 revisions resets a subscription
/// that has not drained, and another made after it is told them all.
#[test]
fn a_subscription_past_its_queue_is_reset_for_good() {
    let (_dir, ops) = board();
    let (_, wake) = counted();
    let mut stalled = ops.subscribe(wake);
    wrote(&ops, 1, true, 299);
    assert_eq!(drained(&mut stalled), FeedDrained::Reset);
    let (_, wake) = counted();
    let mut fresh = ops.subscribe(wake);
    wrote(&ops, 2, false, 1);
    assert_eq!(
        drained(&mut stalled),
        FeedDrained::Reset,
        "a reset is for good"
    );
    assert_eq!(
        drained(&mut fresh),
        FeedDrained::Revisions(vec![revision(301)])
    );
}
