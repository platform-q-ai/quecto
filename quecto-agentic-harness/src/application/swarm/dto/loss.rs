//! Loss and death recording (#2277, #1924, #1961): the requests and
//! answers of `QuarantineMember` (`Workbench._quarantine`),
//! `ConfirmMemberDead` (`_confirmed_dead`) and `LoseCoordinator`
//! (`_lose_coordinator`), and the loss observation the board ports read.
//!
//! As in [`super::membership`], `actor` is the member the call acts as and
//! every other argument stays the JSON value the caller passed: Python
//! binds the member untyped, compares it by `==`, and checks the exit kind
//! at run time.
use serde_json::Value;

use super::RunStatusRow;

/// `Workbench._quarantine(member)` as `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuarantineMemberRequest {
    pub actor: String,
    pub member: Value,
}

/// What `_quarantine` decided (telemetry; Python answers `None`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quarantine {
    /// The member is dead, unknown, or already recorded lost since its
    /// latest activation: nothing is written.
    AlreadyLost,
    /// The caller did not launch the member and its launcher lives:
    /// nothing is written.
    NotLauncher,
    /// An authorised observation inside the grace: at most the caller's
    /// first `scope_observed` is written.
    GracePending,
    /// The loss is recorded (`scope_unknown`) and ends the run by loss.
    Recorded,
}

/// `Workbench._confirmed_dead(member, exit='orderly')` as `actor`. `exit`
/// is the value passed: only the text `orderly` or `abrupt` is a kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfirmMemberDeadRequest {
    pub actor: String,
    pub member: Value,
    pub exit: Value,
}

/// What `_confirmed_dead` decided (telemetry; Python answers `None`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeathConfirmation {
    /// The member is unknown or its death is already confirmed.
    AlreadyDead,
    /// The member is dead now and its active work blocked; `coordinator`
    /// when it was the run's coordinator, whose death ends the run by
    /// loss.
    Confirmed { coordinator: bool },
}

/// `Workbench._lose_coordinator()` as `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoseCoordinatorRequest {
    pub actor: String,
}

/// What `_lose_coordinator` answered: the run's columns after the op (as
/// `_status` reads them) and whether a loss was recorded.
#[derive(Clone, Debug, PartialEq)]
pub struct CoordinatorLoss {
    pub run: RunStatusRow,
    pub lost: bool,
}

/// One `scope_observed` event as `_grace_elapsed` reads it: its actor (the
/// text stored, `None` otherwise), its time as stored (a REAL the board
/// writes; NULL, an INTEGER or TEXT only by an edit, which Python reads
/// only once the observation names the member asked about) and its
/// detail's `member` (`null` when the detail names none or is not an
/// object).
#[derive(Clone, Debug, PartialEq)]
pub struct ScopeObservation {
    pub actor: Option<String>,
    pub time: Value,
    pub member: Value,
}
