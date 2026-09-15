//! The narrow session-runtime ports the save transaction snapshots from
//! (#1860, D5 #1972): what the loop's runtime knows that the session file
//! records besides the conversation itself. Each is an observation of an
//! external effect owner — the agent loop's pruning latch, the workflow
//! engine, the sub-agent registry — never a mutation of it; the transaction
//! owns every state change.
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
