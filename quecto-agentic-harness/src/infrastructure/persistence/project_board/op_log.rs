//! The project board's `board_op` records (epic #2481 T2): the
//! [`ProjectBoardOpLog`] port over the event log's synchronous line handle
//! ([`EventLogProjectBoardOps`]), and the `tracing` record of each op
//! ([`TracingProjectBoardOps`]), which wraps it.
//!
//! Both are built only inside a `BoardRecording`, which exists only while
//! the event log is on, so with it off nothing is written to the log or to
//! `tracing`. This file is the only one of the board's modules that writes
//! `tracing` (an architecture rule).
//!
//! A board request runs on a blocking thread (the tool's) or the CLI's own
//! thread, so its record is appended there, as the swarm board's
//! `swarm_op` is (`board_op_log.rs`): one `write` of one line under the
//! log's write gate, filed under no turn, sharing the log's cap. The
//! record is observational, so it never holds up the board: it waits for
//! the gate at most [`BOARD_OP_GATE_WAIT`]; past that it is dropped and
//! counted, and the next record written is preceded, in the same `write`,
//! by a `board_ops_dropped` line giving the count. Drops never followed by
//! a written record (the log caps, or the process ends first) go unnoted.
//! A record refused at the cap, or a failed write, is dropped too; the
//! first drop of any kind raises one warning for the adapter's lifetime.
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use crate::application::project_board::ports::ProjectBoardOpLog;
use crate::domain::audit::AuditEvent;
use crate::domain::project_board::value_objects::telemetry::{Access, BoardOpRecord};
use crate::infrastructure::persistence::audit_log::AuditCrashLine;

/// The tracing target of the board's records.
const TARGET: &str = "quecto::project_board";

/// How long a `board_op` record waits for the event log's write gate
/// before it is dropped: a writer holds the gate for one `write`, so a
/// wait this long means the log is stuck, and the request goes on.
pub const BOARD_OP_GATE_WAIT: Duration = Duration::from_millis(50);

/// [`ProjectBoardOpLog`] over an event log: a session's, or the CLI's
/// daily `board-cli:<date>` log.
#[derive(Debug)]
pub struct EventLogProjectBoardOps {
    line: AuditCrashLine,
    warned: AtomicBool,
    /// Records dropped at the gate and not yet noted in the log.
    dropped: AtomicU64,
}

impl EventLogProjectBoardOps {
    pub fn new(line: AuditCrashLine) -> Self {
        Self {
            line,
            warned: AtomicBool::new(false),
            dropped: AtomicU64::new(0),
        }
    }

    /// Count `more` dropped at the gate, saturating.
    fn count_dropped(&self, more: u64) {
        // A compare-exchange loop, as `board_op_log.rs` counts (no
        // `fetch_update`, deprecated, nor `try_update`, above the MSRV).
        let mut held = self.dropped.load(Ordering::Acquire);
        while let Err(now) = self.dropped.compare_exchange_weak(
            held,
            held.saturating_add(more),
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            held = now;
        }
    }

    fn warn(&self, error: &std::io::Error) {
        if self.warned.swap(true, Ordering::AcqRel) {
            return;
        }
        tracing::warn!(
            target: TARGET,
            %error,
            "board_op records are not being written to the event log"
        );
    }
}

impl ProjectBoardOpLog for EventLogProjectBoardOps {
    /// Filed under no turn: a board request knows none. Preceded by the
    /// note of the records dropped since the last one written, when any
    /// were.
    fn record(&self, record: BoardOpRecord) {
        // Taken, not read: a concurrent record notes only what it took,
        // and what a record held off the gate took is put back below.
        let unnoted = self.dropped.swap(0, Ordering::AcqRel);
        let mut events = Vec::with_capacity(2);
        if unnoted > 0 {
            events.push(AuditEvent::BoardOpsDropped { dropped: unnoted });
        }
        events.push(AuditEvent::BoardOp(Box::new(record)));
        match self.line.append(None, events, BOARD_OP_GATE_WAIT) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                // Nothing was written: this record, and the drops it was to
                // note, wait for the next record that is.
                self.count_dropped(unnoted.saturating_add(1));
                self.warn(&error);
            }
            // The log is capped (nothing follows its cap record), or the
            // write failed, part of it perhaps landed: these drops go
            // unnoted rather than be counted twice.
            Err(error) => self.warn(&error),
        }
    }
}

/// [`ProjectBoardOpLog`] that writes each record to `tracing` (target
/// `quecto::project_board`, `board op`: DEBUG for a read, INFO for a
/// write) and then to the log it wraps. The `tracing` record holds exactly
/// the fields of the `board_op` line, so it holds no board text either.
#[derive(Debug)]
pub struct TracingProjectBoardOps<L> {
    inner: L,
}

impl<L: ProjectBoardOpLog> TracingProjectBoardOps<L> {
    /// Each record traced, then written to `inner`.
    pub fn over(inner: L) -> Self {
        Self { inner }
    }
}

impl<L: ProjectBoardOpLog> ProjectBoardOpLog for TracingProjectBoardOps<L> {
    fn record(&self, record: BoardOpRecord) {
        let op = record.op.as_str();
        // The fields are rendered only when a subscriber takes the record.
        let fields = Fields(&record);
        match record.access {
            Access::Read => tracing::debug!(target: TARGET, op, record = %fields, "board op"),
            Access::Write => tracing::info!(target: TARGET, op, record = %fields, "board op"),
        }
        self.inner.record(record);
    }
}

/// A record as its `board_op` line's JSON fields.
struct Fields<'a>(&'a BoardOpRecord);

impl std::fmt::Display for Fields<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match serde_json::to_string(self.0) {
            Ok(json) => f.write_str(&json),
            // Never a panic on a board request's thread.
            Err(_) => f.write_str("{\"unrenderable\":true}"),
        }
    }
}

#[cfg(test)]
#[path = "op_log_tests.rs"]
mod tests;
