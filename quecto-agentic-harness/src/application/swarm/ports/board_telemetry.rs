//! The coordination board's call metering and operation log.

use super::board_repository::BoardRepository;
use crate::application::swarm::dto::{CallMeasure, DroppedRecords};
use crate::domain::swarm::events::run_summary::SwarmRunSummary;
use crate::domain::swarm::events::telemetry::BoardOpObservation;
use std::sync::Arc;

/// Port: measures board calls (#2303), one [`MeteredCall`] per call. The
/// dispatcher opens one around a call only while the event log is on
/// (owner decision T1).
///
/// Nothing is ambient: a call is measured only through its own
/// [`MeteredCall`], which is the repository the call runs its transactions
/// through, so no call's transactions, on this thread or another, are
/// mixed into another's measure.
pub trait BoardCallMeter: Send + Sync {
    /// A fresh measure for one call, sharing nothing with any other.
    fn open(&self) -> Arc<dyn MeteredCall>;
}

/// Port: one board call's measure (#2303), and the repository whose
/// transactions it counts: each transaction begun through it is measured
/// here, and only those.
pub trait MeteredCall: BoardRepository {
    /// What was measured so far; `None` while no transaction has begun,
    /// so nothing was measured.
    fn measure(&self) -> Option<CallMeasure>;

    /// The call's record has a fixed role (the harness's own op, #2313
    /// review nit): the run's roles are not read for it, only its id.
    fn role_fixed(&self);
}

/// Port: where each board op's `swarm_op` record goes (#2303): the event
/// log, only while it is on.
pub trait BoardOpLog: Send + Sync {
    /// Appends `observation` synchronously: never on an async runtime,
    /// never blocking on one, never panicking. A failed write is the
    /// adapter's to report and never changes the op's answer.
    fn record(&self, observation: BoardOpObservation);

    /// Appends a run's `summary` (#2313), as [`Self::record`] appends a
    /// record: synchronously, never panicking, a failed write the
    /// adapter's to report.
    fn summarize(&self, summary: SwarmRunSummary);
}

/// Port: a session's event log a board records in (#2313): a
/// [`BoardOpLog`] that also notes the records dropped before they reached
/// it, and hands over, when a session switch replaces it, the drops it
/// counted but has not noted yet.
pub trait SessionOpLog: BoardOpLog {
    /// Notes `drops` (#2313 review L4: held past their bound before the
    /// session's log opened, or counted by the log this one replaced):
    /// written as the log's drop notes, as synchronously and safely as a
    /// record.
    fn dropped(&self, drops: DroppedRecords);

    /// The drops this log counted and has not noted yet, taken (the counts
    /// restart): handed to the log that replaces it on a session switch
    /// (#2313 review nit), so no drop is lost with the departing log.
    fn take_unnoted(&self) -> DroppedRecords;
}
