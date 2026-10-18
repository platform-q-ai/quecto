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
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use crate::application::project_board::ports::ProjectBoardOpLog;
use crate::domain::audit::AuditEvent;
use crate::domain::project_board::value_objects::telemetry::{
    Access, BoardOpRecord, BoardStartRecord, FoldBackRecord,
};
use crate::infrastructure::observability::audit_log::AuditCrashLine;

/// The tracing target of the board's records.
const TARGET: &str = "quecto::project_board";

/// How long a `board_op` record waits for the event log's write gate
/// before it is dropped: a writer holds the gate for one `write`, so a
/// wait this long means the log is stuck, and the request goes on.
pub const BOARD_OP_GATE_WAIT: Duration = Duration::from_millis(50);

/// Opens the line of a log written to only once a record arrives.
type Opener = Box<dyn Fn() -> Option<AuditCrashLine> + Send + Sync>;

/// The line records are appended through.
enum Line {
    /// Already open: a session's log.
    Given(AuditCrashLine),
    /// Opened by the first record, once, whatever it gives: the CLI's
    /// daily log, so a run that records nothing leaves no file.
    OnFirstRecord {
        open: Opener,
        opened: OnceLock<Option<AuditCrashLine>>,
    },
}

impl Line {
    fn get(&self) -> Option<&AuditCrashLine> {
        match self {
            Self::Given(line) => Some(line),
            Self::OnFirstRecord { open, opened } => opened.get_or_init(open).as_ref(),
        }
    }
}

/// [`ProjectBoardOpLog`] over an event log: a session's, or the CLI's
/// daily `board-cli:<date>` log.
pub struct EventLogProjectBoardOps {
    line: Line,
    warned: AtomicBool,
    /// Records dropped at the gate and not yet noted in the log.
    dropped: AtomicU64,
}

impl EventLogProjectBoardOps {
    /// Records appended through `line`, a log already open.
    pub fn new(line: AuditCrashLine) -> Self {
        Self::over(Line::Given(line))
    }

    /// Records appended through the line `open` gives the first record,
    /// called once; when it gives none, every record is dropped, with the
    /// one warning.
    pub fn opened_by_first_record(
        open: impl Fn() -> Option<AuditCrashLine> + Send + Sync + 'static,
    ) -> Self {
        Self::over(Line::OnFirstRecord {
            open: Box::new(open),
            opened: OnceLock::new(),
        })
    }

    fn over(line: Line) -> Self {
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

    fn warn(&self, error: &dyn std::fmt::Display) {
        if self.warned.swap(true, Ordering::AcqRel) {
            return;
        }
        tracing::warn!(
            target: TARGET,
            %error,
            "board_op records are not being written to the event log"
        );
    }

    /// Appends `event`, filed under no turn, preceded by the note of the
    /// records dropped since the last one written, when any were.
    fn append(&self, event: AuditEvent) {
        // Taken, not read: a concurrent record notes only what it took,
        // and what a record held off the gate took is put back below.
        let unnoted = self.dropped.swap(0, Ordering::AcqRel);
        let mut events = Vec::with_capacity(2);
        if unnoted > 0 {
            events.push(AuditEvent::BoardOpsDropped { dropped: unnoted });
        }
        events.push(event);
        let appended = match self.line.get() {
            Some(line) => line.append(None, events, BOARD_OP_GATE_WAIT),
            // Nothing to note the drops in: they go unnoted.
            None => Err(std::io::Error::other("the event log could not be opened")),
        };
        match appended {
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

impl ProjectBoardOpLog for EventLogProjectBoardOps {
    /// Filed under no turn: a board request knows none. Preceded by the
    /// note of the records dropped since the last one written, when any
    /// were.
    fn record(&self, record: BoardOpRecord) {
        self.append(AuditEvent::BoardOp(Box::new(record)));
    }

    /// As [`Self::record`]: a fold-back's record shares the log's gate,
    /// its bound and its count of drops.
    fn record_fold_back(&self, record: FoldBackRecord) {
        self.append(AuditEvent::BoardFoldBack(Box::new(record)));
    }

    fn record_start(&self, record: BoardStartRecord) {
        self.append(AuditEvent::BoardStart(Box::new(record)));
    }
}

/// [`ProjectBoardOpLog`] that writes each record to `tracing` (target
/// `quecto::project_board`, `board op`: DEBUG for a read, INFO for a
/// write) and then to the event log it wraps, only while that log has a
/// line to write to: a daily log that could not be opened traces no
/// record, as a session with no line gives no recording at all. The
/// `tracing` record holds the op and the record's own fields as the
/// `board_op` line serializes them, without the line's envelope (`ts`,
/// `session`, `host`, `pid`, …), so it holds no board text either.
pub struct TracingProjectBoardOps {
    inner: EventLogProjectBoardOps,
}

impl TracingProjectBoardOps {
    /// Each record traced, then written to `inner`.
    pub fn over(inner: EventLogProjectBoardOps) -> Self {
        Self { inner }
    }
}

impl ProjectBoardOpLog for TracingProjectBoardOps {
    fn record(&self, record: BoardOpRecord) {
        if self.inner.line.get().is_some() {
            let op = record.op.as_str();
            // The fields are rendered only when a subscriber takes them.
            let fields = Fields(&record);
            match record.access {
                Access::Read => tracing::debug!(target: TARGET, op, record = %fields, "board op"),
                Access::Write => tracing::info!(target: TARGET, op, record = %fields, "board op"),
            }
        }
        self.inner.record(record);
    }

    /// Traced at INFO (`board fold back`: a fold-back writes the board),
    /// then written as [`Self::record`] writes a `board_op`.
    fn record_fold_back(&self, record: FoldBackRecord) {
        if self.inner.line.get().is_some() {
            let fields = Fields(&record);
            tracing::info!(target: TARGET, record = %fields, "board fold back");
        }
        self.inner.record_fold_back(record);
    }

    fn record_start(&self, record: BoardStartRecord) {
        if self.inner.line.get().is_some() {
            let fields = Fields(&record);
            tracing::info!(target: TARGET, record = %fields, "board start");
        }
        self.inner.record_start(record);
    }
}

/// A record as its line's JSON fields.
struct Fields<'a, T: serde::Serialize>(&'a T);

impl<T: serde::Serialize> std::fmt::Display for Fields<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match serde_json::to_string(self.0) {
            Ok(json) => f.write_str(&json),
            // Never a panic on a board request's thread.
            Err(_) => f.write_str("{\"unrenderable\":true}"),
        }
    }
}

#[cfg(test)]
#[path = "project_board_op_log_tests.rs"]
mod tests;
