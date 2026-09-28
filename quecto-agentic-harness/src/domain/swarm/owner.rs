//! Owner-liveness policy ported from `swarm_policy.py` (#2267, #1969): the
//! store's affirmative, read-side view of a task owner.

/// Every state [`owner_state`] names.
pub const OWNER_STATES: [&str; 6] = ["active", "idle", "reserved", "lost", "dead", "unknown"];
/// A live task owner with no board event for this long, in seconds, reads as
/// `idle` (#1969). It is a prompt to look, not a stall.
pub const OWNER_IDLE_AFTER: f64 = 300.0;
/// Owner states `send` accepts as a recipient; the others get `recovery`.
pub const ADDRESSABLE_OWNER_STATES: [&str; 2] = ["active", "idle"];

/// The store's view of a task owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerState {
    Active,
    Idle,
    Reserved,
    Lost,
    Dead,
    Unknown,
}

impl OwnerState {
    pub fn as_str(self) -> &'static str {
        "unknown"
    }

    /// Whether `send` accepts an owner in this state as a recipient.
    pub fn addressable(self) -> bool {
        false
    }
}

/// The owner's state from its member status, whether a lost-scope record is
/// newer than its latest activation, and its most recent board event.
pub fn owner_state(
    _member_status: Option<&str>,
    _lost: bool,
    _last_activity: Option<f64>,
    _now: f64,
    _idle_after: f64,
) -> OwnerState {
    OwnerState::Unknown
}

/// How the coordinator moves work off an owner `send` cannot reach.
pub fn owner_recovery(_state: OwnerState) -> &'static str {
    ""
}

/// When a live owner last active at `last_activity` turns idle; `None` once
/// it already has.
pub fn idle_transition(_last_activity: Option<f64>, _now: f64, _idle_after: f64) -> Option<f64> {
    None
}

#[cfg(test)]
#[path = "owner_tests.rs"]
mod owner_tests;
