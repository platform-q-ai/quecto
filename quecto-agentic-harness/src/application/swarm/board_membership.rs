//! How the membership use cases (#2271) read a member's row: the checks
//! `_activate`, `_record_launch`, `_release_unlaunched` and `join_process`
//! make on `dict(row)`, with Python's comparisons. Capability-internal
//! helpers, not a use case and not a port.
use serde_json::Value;

use super::dto::{LaunchIdentity, MemberRow};

/// `bounded(member, 'member', 128)`: the longest member id, in UTF-8 bytes.
pub(crate) const MEMBER_MAX_BYTES: usize = 128;

/// A member whose death is not confirmed: `live` or `reserved`. Python
/// refuses only `'dead'`; any other status (NULL, or one the board never
/// writes) is refused here too, the owner-decided divergence
/// `unknown_member_status_is_not_alive` (#2295).
pub(crate) fn alive(row: &MemberRow) -> bool {
    matches!(row.text("status"), Some("live" | "reserved"))
}

/// The member's row is `live`.
pub(crate) fn live(row: &MemberRow) -> bool {
    matches!(row.text("status"), Some("live"))
}

/// A `reserved` row with no process recorded: an admission never launched.
pub(crate) fn unlaunched(row: &MemberRow) -> bool {
    matches!(row.text("status"), Some("reserved")) && matches!(row.get("pid"), Some(Value::Null))
}

/// `row['reservation'] == reservation`: text equals the same text, and a
/// NULL reservation equals only an absent one (`None == None`).
pub(crate) fn holds_reservation(row: &MemberRow, reservation: Option<&str>) -> bool {
    match (row.get("reservation"), reservation) {
        (Some(Value::String(held)), Some(given)) => held == given,
        (Some(Value::Null), None) => true,
        _ => false,
    }
}

/// `row['pid'] == pid and row['started'] == started`: the row names this
/// process. A stored pid equals the integer as a number (`7.0 == 7`), never
/// as text; the start time equals only the same text.
pub(crate) fn same_process(row: &MemberRow, launch: &LaunchIdentity) -> bool {
    row.get("pid").is_some_and(|pid| same_pid(pid, launch.pid))
        && row.text("started") == Some(launch.started.as_str())
}

/// `row['pid'] is not None and (row['pid'], row['started']) != (pid, started)`:
/// a process is recorded and it is another.
pub(crate) fn launched_elsewhere(row: &MemberRow, launch: &LaunchIdentity) -> bool {
    match row.get("pid") {
        Some(Value::Null) | None => false,
        Some(_) => !same_process(row, launch),
    }
}

/// Python's `stored == pid` for an INTEGER or REAL cell: exact, so a float
/// equals only the integer it is (`2.0**53 != 2**53 + 1`).
fn same_pid(stored: &Value, pid: i64) -> bool {
    match stored {
        Value::Number(number) => match (number.as_i64(), number.as_f64()) {
            (Some(integer), _) => integer == pid,
            // An integral float within i64 converts exactly, and converting
            // back finds it again; one beyond i64 saturates and does not.
            (None, Some(float)) => {
                float.fract() == 0.0 && float as i64 == pid && (float as i64) as f64 == float
            }
            (None, None) => false,
        },
        Value::Null | Value::Bool(_) | Value::String(_) | Value::Array(_) | Value::Object(_) => {
            false
        }
    }
}

#[cfg(test)]
#[path = "board_membership_tests.rs"]
mod tests;
