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
        match self {
            Self::Active => "active",
            Self::Idle => "idle",
            Self::Reserved => "reserved",
            Self::Lost => "lost",
            Self::Dead => "dead",
            Self::Unknown => "unknown",
        }
    }

    /// Whether `send` accepts an owner in this state as a recipient
    /// ([`ADDRESSABLE_OWNER_STATES`]).
    pub fn addressable(self) -> bool {
        match self {
            Self::Active | Self::Idle => true,
            Self::Reserved | Self::Lost | Self::Dead | Self::Unknown => false,
        }
    }
}

/// The store's affirmative view of a task owner (#1969), read-side only.
///
/// Authoritative store signals first: `dead` (the launcher's harness
/// confirmed the exit), `lost` (a `scope_unknown` record newer than the
/// member's latest activation), `reserved` (admitted, never launched). Only a
/// `live` member reads `active`/`idle` from its most recent board event
/// (`last_activity`, Unix seconds) against the store clock `now`. Every other
/// status, and a live member without an event, is `unknown`. A provider
/// suspension is a harness fact the store cannot see and is never claimed.
pub fn owner_state(
    member_status: Option<&str>,
    lost: bool,
    last_activity: Option<f64>,
    now: f64,
    idle_after: f64,
) -> OwnerState {
    match (member_status, lost, last_activity) {
        (Some("dead"), _, _) => OwnerState::Dead,
        (Some("live" | "reserved"), true, _) => OwnerState::Lost,
        (Some("reserved"), false, _) => OwnerState::Reserved,
        (Some("live"), false, Some(at)) if now - at >= idle_after => OwnerState::Idle,
        (Some("live"), false, Some(_)) => OwnerState::Active,
        _ => OwnerState::Unknown,
    }
}

/// How the coordinator moves work off an owner `send` cannot reach.
pub fn owner_recovery(state: OwnerState) -> &'static str {
    match state {
        OwnerState::Dead => "recover(task) or revoke(task, reason)",
        OwnerState::Lost => {
            "resume the run (agent_cmd swarm_control resume), then revoke(task, reason)"
        }
        OwnerState::Active | OwnerState::Idle | OwnerState::Reserved | OwnerState::Unknown => {
            "revoke(task, reason)"
        }
    }
}

/// When a live owner last active at `last_activity` turns idle: that instant
/// while it is still after `now`, else `None` (read side; nothing schedules
/// anything on it).
pub fn idle_transition(last_activity: Option<f64>, now: f64, idle_after: f64) -> Option<f64> {
    let at = last_activity? + idle_after;
    (at > now).then_some(at)
}

#[cfg(test)]
#[path = "owner_tests.rs"]
mod owner_tests;
