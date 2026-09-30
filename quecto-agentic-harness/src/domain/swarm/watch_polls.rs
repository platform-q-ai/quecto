//! The run watch's ticks, as the event log keeps them (#2338).
//!
//! Every member's harness watches its run: each tick is one `_watch` call,
//! which answers `unchanged` while the board's event cursor is the one the
//! watch passed, and the run's snapshot otherwise
//! (`domain::swarm::watch`). An `unchanged` tick is still a board call,
//! and the event log still accounts for it, but not one record per tick:
//! consecutive `unchanged` ticks of one run are folded into one `swarm_op`
//! record whose `decision` is [`UNCHANGED`] and whose `polls` counts them.
//! Any other tick (one that answered the snapshot, one the busy handler
//! slowed, one that was refused) is recorded alone, as every other board
//! call is.
//!
//! An aggregate is written when the next tick is not absorbed, when it has
//! been open for [`POLL_RECORD_PERIOD_US`], or when it is flushed (the
//! watch ends, the run is summarised, recording stops, the handles holding
//! it are dropped, the harness exits). Its measures are its slowest
//! tick's: `duration_us`, `lock_wait_us`, `busy_wait_us`, `commit_us`
//! and `result_bytes` are each the maximum over the ticks it holds, and `busy`
//! is `false` (a busy tick is never absorbed). It carries what every
//! record carries, ids, kinds, durations and sizes, and no text.
use super::telemetry::{BoardOpObservation, BoardOpOutcome};

/// The watch's tick, by its board method name.
pub const WATCH_POLL_OP: &str = "_watch";

/// The decision of a tick that found the cursor unchanged, and of an
/// aggregate of such ticks.
pub const UNCHANGED: &str = "unchanged";

/// The decision of a tick that answered the run's snapshot.
pub const SNAPSHOT: &str = "snapshot";

/// The longest an aggregate stays open before it is written, in
/// microseconds.
pub const POLL_RECORD_PERIOD_US: u64 = 60_000_000;

/// The watch's `unchanged` ticks not yet written.
#[derive(Debug, Default)]
pub struct PollTally {
    /// The open aggregate, and when (on the caller's clock) it opened.
    pending: Option<(BoardOpObservation, u64)>,
}

impl PollTally {
    /// A watch tick, `observation`, finished at `now_us` on the caller's
    /// monotonic clock. Returns the records to write now, in order: none
    /// while the tick is absorbed into the open aggregate; otherwise the
    /// aggregate it closes (if any), then the tick itself.
    pub fn poll(
        &mut self,
        observation: BoardOpObservation,
        now_us: u64,
    ) -> Vec<BoardOpObservation> {
        debug_assert_eq!(observation.op, WATCH_POLL_OP, "only a watch tick");
        debug_assert!(
            observation.detail.polls.is_none(),
            "a tick is one board call, never an aggregate"
        );
        let unchanged = matches!(observation.outcome, BoardOpOutcome::Ok)
            && observation.busy == Some(false)
            && observation.decision.as_deref() == Some(UNCHANGED);
        let same_run = self.pending.as_ref().is_none_or(|(aggregate, _)| {
            aggregate.run_id == observation.run_id && aggregate.actor_ref == observation.actor_ref
        });
        match (unchanged, same_run) {
            (true, true) => self.absorb(observation, now_us),
            (true, false) => {
                let closed = self.flush();
                closed
                    .into_iter()
                    .chain(self.absorb(observation, now_us))
                    .collect()
            }
            (false, _) => {
                let mut records: Vec<_> = self.flush().into_iter().collect();
                records.push(observation);
                records
            }
        }
    }

    /// The open aggregate, if any, taken to be written.
    pub fn flush(&mut self) -> Option<BoardOpObservation> {
        let (aggregate, _) = self.pending.take()?;
        debug_assert!(
            aggregate.detail.polls.is_some_and(|polls| polls >= 1),
            "an aggregate holds at least one poll"
        );
        Some(aggregate)
    }

    /// The polls the open aggregate holds (0 when none is open).
    pub fn pending_polls(&self) -> u64 {
        self.pending
            .as_ref()
            .and_then(|(aggregate, _)| aggregate.detail.polls)
            .unwrap_or(0)
    }

    /// Folds an unchanged poll into the open aggregate (opening one when
    /// none is open), and writes the aggregate once it has been open for
    /// [`POLL_RECORD_PERIOD_US`].
    fn absorb(&mut self, poll: BoardOpObservation, now_us: u64) -> Vec<BoardOpObservation> {
        match self.pending.as_mut() {
            Some((aggregate, _)) => merge(aggregate, &poll),
            None => self.pending = Some((opened(poll), now_us)),
        }
        let due = self
            .pending
            .as_ref()
            .is_some_and(|(_, since)| now_us.saturating_sub(*since) >= POLL_RECORD_PERIOD_US);
        match due {
            true => self.flush().into_iter().collect(),
            false => Vec::new(),
        }
    }
}

/// An aggregate opened on its first unchanged tick.
fn opened(mut poll: BoardOpObservation) -> BoardOpObservation {
    debug_assert_eq!(poll.decision.as_deref(), Some(UNCHANGED));
    poll.cursor_moved = Some(false);
    poll.detail.polls = Some(1);
    poll
}

/// Adds `poll` to `aggregate`: one more poll, each measure the slower's.
fn merge(aggregate: &mut BoardOpObservation, poll: &BoardOpObservation) {
    debug_assert_eq!(aggregate.run_id, poll.run_id, "one run per aggregate");
    debug_assert_eq!(aggregate.actor_ref, poll.actor_ref, "one caller");
    aggregate.detail.polls = aggregate.detail.polls.map(|polls| polls.saturating_add(1));
    aggregate.duration_us = aggregate.duration_us.max(poll.duration_us);
    aggregate.lock_wait_us = slower(aggregate.lock_wait_us, poll.lock_wait_us);
    aggregate.busy_wait_us = slower(aggregate.busy_wait_us, poll.busy_wait_us);
    aggregate.commit_us = slower(aggregate.commit_us, poll.commit_us);
    aggregate.result_bytes = aggregate.result_bytes.max(poll.result_bytes);
}

/// The larger of two measures; a measure is kept over an unmeasured one.
fn slower(held: Option<u64>, other: Option<u64>) -> Option<u64> {
    match (held, other) {
        (Some(held), Some(other)) => Some(held.max(other)),
        (Some(measured), None) | (None, Some(measured)) => Some(measured),
        (None, None) => None,
    }
}

#[cfg(test)]
#[path = "watch_polls_tests.rs"]
mod tests;
