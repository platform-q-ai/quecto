//! The board's `swarm_op` records in the event log (#2303): the
//! [`BoardOpLog`] port over the session log's synchronous line handle
//! ([`AuditCrashLine::append`]).
//!
//! A board call runs synchronously on its own thread (a blocking thread
//! under a runtime, or a plain thread), so its record is appended there:
//! one `write` of one line on the log's `O_APPEND` handle, no async
//! runtime, no lock, no panic. The line shares the log's cap budget with
//! the async writer, so the cap stays exact; a line refused at the cap, or
//! a failed write, is dropped with one warning for the log's lifetime and
//! never touches the op's answer. The async writer takes no lock this
//! handle waits on, so a `swarm_op` line and an agent record written at
//! the same instant are two `write`s on one append-mode file: each lands
//! whole on a local file system, though POSIX does not promise that for a
//! regular file (as for the crash line).
use std::sync::atomic::{AtomicBool, Ordering};

use crate::application::swarm::ports::BoardOpLog;
use crate::domain::audit::AuditEvent;
use crate::domain::swarm::BoardOpObservation;
use crate::infrastructure::persistence::audit_log::AuditCrashLine;

/// The tracing target of the one warning a failed record raises.
const TARGET: &str = "quecto::swarm_board";

/// [`BoardOpLog`] over a session's event log.
#[derive(Debug)]
pub struct EventLogBoardOps {
    line: AuditCrashLine,
    warned: AtomicBool,
}

impl EventLogBoardOps {
    pub fn new(line: AuditCrashLine) -> Self {
        Self {
            line,
            warned: AtomicBool::new(false),
        }
    }
}

impl BoardOpLog for EventLogBoardOps {
    /// Filed under no turn: the board call knows none.
    fn record(&self, observation: BoardOpObservation) {
        // Red stub (#2303): writes nothing.
        let written: std::io::Result<()> = {
            let _ = (&self.line, AuditEvent::SwarmOp(observation));
            Ok(())
        };
        if let Err(error) = written {
            if !self.warned.swap(true, Ordering::AcqRel) {
                tracing::warn!(
                    target: TARGET,
                    %error,
                    "swarm_op records are not being written to the event log"
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "board_op_log_tests.rs"]
mod tests;
