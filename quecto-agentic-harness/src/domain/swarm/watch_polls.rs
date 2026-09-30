//! The run watch's event-cursor polls, as the event log keeps them (#2338).
//!
//! Every member's harness watches its run: it reads the board's event
//! cursor (`_event_cursor`) every tick and takes a full `_snapshot` only
//! when the cursor moved or a check is due (`domain::swarm::watch`).
//! A tick whose cursor did not move is still a board call, and the event
//! log still accounts for it, but not one record per tick: consecutive
//! unchanged polls of one run are folded into one `swarm_op` record whose
//! `decision` is [`UNCHANGED`] and whose `polls` counts them. Any other
//! watch poll (the first, one that found the cursor moved, one the busy
//! handler slowed, one that was refused) is recorded alone, as every other
//! board call is.
//!
//! An aggregate is written when the next poll is not absorbed, when the
//! watch records anything else (its snapshot), when it has been open for
//! [`POLL_RECORD_PERIOD_US`], or when it is flushed (the watch ends, the
//! run is summarised, the handles holding it are dropped). Its measures are
//! its slowest poll's: `duration_us`, `lock_wait_us`, `busy_wait_us` and
//! `result_bytes` are each the maximum over the polls it holds, and `busy`
//! is `false` (a busy poll is never absorbed). It carries what every
//! record carries, ids, kinds, durations and sizes, and no text.
use super::telemetry::{BoardOpObservation, BoardOpOutcome};

/// The watch's cursor read, by its board method name.
pub const WATCH_POLL_OP: &str = "_event_cursor";

/// The decision an aggregate of unchanged polls records.
pub const UNCHANGED: &str = "unchanged";

/// The longest an aggregate stays open before it is written, in
/// microseconds: a process that ends without flushing loses at most this
/// long a count.
pub const POLL_RECORD_PERIOD_US: u64 = 60_000_000;

/// The watch's polls not yet written, and the cursor its latest poll read.
#[derive(Debug, Default)]
pub struct PollTally {
    /// The run and cursor the watch's latest answered poll read.
    last: Option<(Option<String>, i64)>,
    /// The open aggregate, and when (on the caller's clock) it opened.
    pending: Option<(BoardOpObservation, u64)>,
}

impl PollTally {
    /// A watch poll, `observation`, that answered `cursor` (`None` when it
    /// was refused), finished at `now_us` on the caller's monotonic clock.
    /// Returns the records to write now, in order: none while the poll is
    /// absorbed into the open aggregate; otherwise the aggregate it closes
    /// (if any), then the poll itself.
    pub fn poll(
        &mut self,
        observation: BoardOpObservation,
        cursor: Option<i64>,
        now_us: u64,
    ) -> Vec<BoardOpObservation> {
        debug_assert_eq!(observation.op, WATCH_POLL_OP, "only a cursor poll");
        debug_assert!(
            observation.detail.polls.is_none(),
            "a poll is one board call, never an aggregate"
        );
        let answered = matches!(observation.outcome, BoardOpOutcome::Ok);
        let unchanged = match (&self.last, cursor, answered, observation.busy) {
            (Some((run, last)), Some(cursor), true, Some(false)) => {
                *last == cursor && *run == observation.run_id
            }
            _ => false,
        };
        if let Some(cursor) = cursor.filter(|_| answered) {
            self.last = Some((observation.run_id.clone(), cursor));
        }
        match unchanged {
            true => self.absorb(observation, now_us),
            false => {
                let mut records: Vec<_> = self.flush().into_iter().collect();
                records.push(observation);
                records
            }
        }
    }

    /// The watch records something else (its snapshot): the open
    /// aggregate, if any, is written first, so the log keeps the order the
    /// calls were made in.
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

/// An aggregate opened on its first unchanged poll.
fn opened(mut poll: BoardOpObservation) -> BoardOpObservation {
    poll.decision = Some(UNCHANGED.to_owned());
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
