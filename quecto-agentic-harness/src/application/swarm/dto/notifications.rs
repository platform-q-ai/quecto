//! Wake notifications (#2276): the requests and answers of
//! `ClaimNotifications` (`Workbench._notifications`) and `AcceptWake`
//! (`Workbench._accept_wake`), and the cursor the board ports advance.
//!
//! As in [`super::messages`], `actor` is the member the call acts as and
//! every other argument stays the JSON value the caller passed: Python
//! takes `with_generation` by its truth and type-checks `generation` at
//! run time.
use serde_json::Value;

use super::MemberRow;

/// `Workbench._notifications(with_generation=False)` as `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimNotificationsRequest {
    pub actor: String,
    pub with_generation: Value,
}

/// What `_notifications` answered: the members woken, as `dict(row)`
/// sorted by id, and the generation its cursor now holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationBatch {
    pub members: Vec<MemberRow>,
    pub generation: i64,
    /// Python's truth of `with_generation`: the answer is
    /// `{members, generation}`, not the members alone.
    pub with_generation: bool,
    /// Whether the caller's notification cursor moved (telemetry).
    pub cursor_moved: bool,
}

/// `Workbench._accept_wake(generation)` as `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcceptWakeRequest {
    pub actor: String,
    pub generation: Value,
}

/// What `_accept_wake` answered: whether the claimed events still wake
/// the caller, and whether its wake cursor moved (telemetry).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WakeAccepted {
    pub woken: bool,
    pub cursor_moved: bool,
}

/// A notification cursor `advance_notifications` wrote: the integer it
/// held before (`None` without a row, or for a value that is not an
/// integer) and the one it holds now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotificationCursor {
    pub previous: Option<i64>,
    pub current: i64,
}

impl NotificationCursor {
    /// Whether the write changed the cursor.
    pub fn moved(&self) -> bool {
        self.previous != Some(self.current)
    }
}
