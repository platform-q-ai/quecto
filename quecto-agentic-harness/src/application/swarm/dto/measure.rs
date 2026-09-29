//! What one board call's transactions measured (#2303): the value the
//! [`BoardCallMeter`](crate::application::swarm::ports::BoardCallMeter)
//! port hands back and the dispatcher's records are built from.
use std::time::Duration;

/// What one board call's transactions measured (#2303): only the
/// transactions the call began through its own metered repository.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CallMeasure {
    /// How many board transactions the call began (at least one: a call
    /// that began none measured nothing, and has no measure).
    pub transactions: u32,
    /// From `BEGIN IMMEDIATE` issued to acquired (or given up), summed
    /// over the call's transactions.
    pub lock_wait: Duration,
    /// The time the store's busy handler slept, summed over every
    /// statement of the call that found the database busy: `BEGIN`, a
    /// read or the commit.
    pub busy_wait: Duration,
    /// Whether the busy handler fired at all.
    pub busy: bool,
    /// The run id the call's first transaction to find one found.
    pub run_id: Option<String>,
    /// The run's coordinator and integrator, as the first run row the call
    /// read holds them (#2303 reconcile): only to name a member-facing
    /// op's caller role, never a field of its record.
    pub run_roles: Option<RunRoles>,
}

/// Who holds a run's two named roles, as its row holds them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunRoles {
    pub coordinator: Option<String>,
    pub integrator: Option<String>,
}
