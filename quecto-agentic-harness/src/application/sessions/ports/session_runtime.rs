//! The narrow session-runtime ports the session transactions reach the
//! loop's runtime through (#1860, D5 #1972; #1864/#1865, D6 #1975): what
//! the runtime knows that the session file records besides the
//! conversation itself, and the per-turn accounting a history replacement
//! resets. The observations — the agent loop's pruning latch, the workflow
//! engine, the sub-agent registry — never mutate their owner; the
//! accounting reset is the one runtime effect the clear and rewind
//! transactions order, and the runtime owns the counters it resets.
use crate::domain::session::PersistedSubagentRosterEntry;
use crate::domain::workflow::WorkflowRunPersisted;

/// Port: the agent loop's durable-prefix dirty latch (#1072). `take` is
/// read-and-clear; the transaction keeps a taken observation sticky in the
/// session state until a save succeeds, so a failed save stays retryable.
pub trait DurablePrefixObservation: Send + Sync {
    fn take_durable_prefix_dirty(&self) -> bool;
}

/// Port: the workflow run to record with the session, if a workflow is
/// bound and has state worth restoring.
pub trait WorkflowRunSource: Send + Sync {
    fn persisted_run(&self) -> Option<WorkflowRunPersisted>;
}

/// Port: the sub-agent roster as history rows (#1937): identity, display
/// name, liveness, status and delivery bookkeeping — never a socket or a
/// pid. Rows come back in registry order with their default restore
/// reason; the transaction stamps the reason and orders them.
pub trait HistoricalRosterSource: Send + Sync {
    fn roster_rows(&self) -> Vec<PersistedSubagentRosterEntry>;
}

/// Port: the loop's per-session turn accounting — the usage counters and
/// context-size reading, the queued pending prompts, and the visible
/// message count its execution view reports. A clear or rewind replaced
/// the history: the runtime zeroes what it accounted for the old
/// transcript, drops the prompts queued against it, and reports
/// `visible_message_count` — the transcript minus the injected prompt —
/// as the new count.
pub trait TurnAccountingReset: Send {
    fn history_replaced(&mut self, visible_message_count: usize);
}
