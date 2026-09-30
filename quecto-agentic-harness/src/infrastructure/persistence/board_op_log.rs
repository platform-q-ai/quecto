//! The board's `swarm_op` records in the event log (#2303): the
//! [`BoardOpLog`] port over the session log's synchronous line handle
//! ([`AuditCrashLine::append`]).
//!
//! A board call runs synchronously on its own thread (a blocking thread
//! under a runtime, or a plain thread), so its record is appended there,
//! after the call's answer is computed and before it is returned: one
//! `write` of one line on the log's `O_APPEND` handle, under the log's
//! write gate, with no async runtime and no panic. Every writer of the log
//! takes that gate for its one write, so a `swarm_op` line never
//! interleaves with an agent record however long either is, and records
//! from one thread land in the order its calls were made. The line shares
//! the log's cap budget with every writer, so the cap stays exact: the
//! first line that does not fit, from whichever writer, is replaced by the
//! log's one `log_capped` record, and nothing is written after it.
//!
//! The record is observational, so it never holds up the board (#2303
//! swarm review): it waits for the gate at most [`SWARM_OP_GATE_WAIT`].
//! Past that bound it is dropped and counted, and the next record written
//! is preceded, in the same `write`, by a `swarm_ops_dropped` line giving
//! the count (and a run summary dropped at its own, longer bound by a
//! `swarm_run_summary_dropped` line, #2313). Drops that are never followed by a written record (the log
//! caps, or the session ends first) go unnoted. A record refused at the
//! cap, or a failed write, is dropped too, and never touches the op's
//! answer; the first drop of any kind raises one warning for the log's
//! lifetime.
//!
//! Writing on the caller's thread, rather than through a queue and writer
//! thread, keeps every record on disk when its call returns and needs no
//! drain at exit. Its residual risk: the `write` itself is not bounded, so
//! a filesystem that never returns from it holds the one call that is
//! writing (every other call then gives up at the gate's bound).
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use crate::application::swarm::dto::DroppedRecords;
use crate::application::swarm::ports::{BoardOpLog, SessionOpLog};
use crate::domain::audit::AuditEvent;
use crate::domain::swarm::{BoardOpObservation, SwarmRunSummary};
use crate::infrastructure::persistence::audit_log::AuditCrashLine;

/// The tracing target of the one warning a failed record raises.
const TARGET: &str = "quecto::swarm_board";

/// How long a `swarm_op` record waits for the event log's write gate
/// before it is dropped: a writer holds the gate for one `write`, so a
/// wait this long means the log is stuck, and the board call goes on.
pub const SWARM_OP_GATE_WAIT: Duration = Duration::from_millis(50);

/// How long a run's `swarm_run_summary` waits for the write gate (#2313
/// review L1): it is written once per run, when the run settles, off every
/// board call's path, so it waits longer than a record before it is
/// dropped (and counted as a record is).
pub const SUMMARY_GATE_WAIT: Duration = Duration::from_secs(2);

/// [`BoardOpLog`] over a session's event log.
#[derive(Debug)]
pub struct EventLogBoardOps {
    line: AuditCrashLine,
    gate_wait: Duration,
    summary_wait: Duration,
    warned: AtomicBool,
    /// Records dropped at the gate and not yet noted in the log.
    dropped: AtomicU64,
    /// Run summaries dropped at the gate and not yet noted (#2313 final
    /// review: counted apart from the records).
    summaries_dropped: AtomicU64,
}

impl EventLogBoardOps {
    pub fn new(line: AuditCrashLine) -> Self {
        Self {
            line,
            gate_wait: SWARM_OP_GATE_WAIT,
            summary_wait: SUMMARY_GATE_WAIT,
            warned: AtomicBool::new(false),
            dropped: AtomicU64::new(0),
            summaries_dropped: AtomicU64::new(0),
        }
    }

    /// The same adapter, waiting at most `bound` for the log's write gate.
    pub fn with_gate_wait(mut self, bound: Duration) -> Self {
        self.gate_wait = bound;
        self
    }

    /// The same adapter, a summary waiting at most `bound` for the gate.
    pub fn with_summary_wait(mut self, bound: Duration) -> Self {
        self.summary_wait = bound;
        self
    }

    /// Count `drops` more dropped at the gate, saturating.
    fn count_dropped(&self, drops: DroppedRecords) {
        for (counter, more) in [
            (&self.dropped, drops.ops),
            (&self.summaries_dropped, drops.summaries),
        ] {
            let _always = counter.fetch_update(Ordering::AcqRel, Ordering::Acquire, |held| {
                Some(held.saturating_add(more))
            });
        }
    }

    fn warn(&self, error: &std::io::Error) {
        if self.warned.swap(true, Ordering::AcqRel) {
            return;
        }
        tracing::warn!(
            target: TARGET,
            %error,
            "swarm_op records are not being written to the event log"
        );
    }
}

impl BoardOpLog for EventLogBoardOps {
    /// Filed under no turn: the board call knows none. Preceded by the
    /// notes of the records and summaries dropped since the last record
    /// written, when any were.
    fn record(&self, observation: BoardOpObservation) {
        // Taken, not read: a concurrent record notes only what it took, and
        // what a record fails to write is put back below.
        let unnoted = self.take_unnoted();
        let mut events = notes(unnoted);
        events.push(AuditEvent::SwarmOp(observation));
        match self.line.append(None, events, self.gate_wait) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                // Nothing was written: this record, and the drops it was
                // to note, wait for the next record that is.
                self.count_dropped(unnoted.plus(DroppedRecords {
                    ops: 1,
                    summaries: 0,
                }));
                self.warn(&error);
            }
            // The log is capped (nothing follows its cap record), or the
            // write failed, part of it perhaps landed: these drops go
            // unnoted rather than be counted twice.
            Err(error) => self.warn(&error),
        }
    }

    /// Filed under no turn, as a record is (#2313). It waits for the gate
    /// up to [`SUMMARY_GATE_WAIT`] (review L1); held off past that, it is
    /// dropped and counted apart from the records (final review), so the
    /// next record written notes it as `swarm_run_summary_dropped`. One the
    /// cap refused or a write failed is dropped with the log's one warning.
    /// It is written once, and never retried.
    fn summarize(&self, summary: SwarmRunSummary) {
        let event = AuditEvent::SwarmRunSummary(summary);
        match self.line.append(None, vec![event], self.summary_wait) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                self.count_dropped(DroppedRecords {
                    ops: 0,
                    summaries: 1,
                });
                self.warn(&error);
            }
            Err(error) => self.warn(&error),
        }
    }
}

impl SessionOpLog for EventLogBoardOps {
    /// The notes of `drops` and the drops not noted yet, now; held off at
    /// the gate, they wait for the next record.
    fn dropped(&self, drops: DroppedRecords) {
        let unnoted = self.take_unnoted().plus(drops);
        if !unnoted.any() {
            return;
        }
        match self.line.append(None, notes(unnoted), self.gate_wait) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                self.count_dropped(unnoted);
                self.warn(&error);
            }
            Err(error) => self.warn(&error),
        }
    }

    fn take_unnoted(&self) -> DroppedRecords {
        DroppedRecords {
            ops: self.dropped.swap(0, Ordering::AcqRel),
            summaries: self.summaries_dropped.swap(0, Ordering::AcqRel),
        }
    }
}

/// The drop notes for `drops`: a `swarm_ops_dropped` line for the records
/// and a `swarm_run_summary_dropped` line for the summaries, each only when
/// there were any.
fn notes(drops: DroppedRecords) -> Vec<AuditEvent> {
    let mut events = Vec::with_capacity(3);
    if drops.ops > 0 {
        events.push(AuditEvent::SwarmOpsDropped { dropped: drops.ops });
    }
    if drops.summaries > 0 {
        events.push(AuditEvent::SwarmRunSummaryDropped {
            dropped: drops.summaries,
        });
    }
    events
}

#[cfg(test)]
#[path = "board_op_log_tests.rs"]
mod tests;
