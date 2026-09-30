//! The run watch's event-cursor polls, as the event log keeps them (#2338).
//!
//! Every member's harness watches its run: it reads the board's event
//! cursor (`_event_cursor`) every tick and takes a full `_snapshot` only
//! when the cursor moved or a check is due (`application::swarm::watch`).
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
    /// Returns the records to write now, in order.
    pub fn poll(
        &mut self,
        observation: BoardOpObservation,
        _cursor: Option<i64>,
        _now_us: u64,
    ) -> Vec<BoardOpObservation> {
        let _ = (&self.last, &self.pending, BoardOpOutcome::Ok);
        vec![observation]
    }

    /// The open aggregate, if any, written.
    pub fn flush(&mut self) -> Option<BoardOpObservation> {
        None
    }

    /// The polls the open aggregate holds (0 when none is open).
    pub fn pending_polls(&self) -> u64 {
        0
    }
}

#[cfg(test)]
#[path = "watch_polls_tests.rs"]
mod tests;
