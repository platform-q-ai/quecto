//! STUB (#2273 red phase): the shared control reads.
use serde_json::Value;

use super::dto::ControlReceipt;
use super::ports::{BoardEvents, BoardMembers, BoardRuns, BoardUsage, Clock};
use crate::domain::swarm::BoardError;

/// # Errors
/// Pending #2273.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn receipt(
    _transaction: &(impl BoardRuns + BoardMembers + BoardEvents + BoardUsage + ?Sized),
    _clock: &dyn Clock,
) -> Result<ControlReceipt, BoardError> {
    Err(BoardError::new("pending #2273"))
}

/// Pending #2273.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn paused_for(_now: f64, _started: f64) -> (f64, Value) {
    (f64::NAN, Value::Null)
}

#[cfg(test)]
#[path = "board_control_tests.rs"]
mod tests;
