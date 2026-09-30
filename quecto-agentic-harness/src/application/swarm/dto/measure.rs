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
    /// The time the call's `COMMIT`s took (#2340), summed over its
    /// transactions, a failed one included: a committed write's journal
    /// and database `fsync`s.
    pub commit: Duration,
    /// The run id the call's first transaction to find one found.
    pub run_id: Option<String>,
    /// The run's coordinator and integrator, as the first run row the call
    /// read holds them (#2303 reconcile): only to name a member-facing
    /// op's caller role, never a field of its record.
    pub run_roles: Option<RunRoles>,
    /// Whether the operation gate authorised the caller as a member of the
    /// run, with the op's own access, in any of the call's transactions
    /// (#2313 review M2): a refusal the op made after it still records the
    /// caller's role.
    pub authorized: bool,
}

/// Who holds a run's two named roles, as its row holds them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunRoles {
    pub coordinator: Option<String>,
    pub integrator: Option<String>,
}

/// Board records dropped before they were written (#2313 final review):
/// `swarm_op` records and run summaries, counted apart.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DroppedRecords {
    pub ops: u64,
    pub summaries: u64,
}

impl DroppedRecords {
    /// Whether anything was dropped.
    pub fn any(self) -> bool {
        self.ops > 0 || self.summaries > 0
    }

    /// Both counts, added, saturating.
    pub fn plus(self, other: Self) -> Self {
        Self {
            ops: self.ops.saturating_add(other.ops),
            summaries: self.summaries.saturating_add(other.summaries),
        }
    }
}
