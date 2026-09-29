//! How the membership use cases (#2271) read a member's row: the checks
//! `_activate`, `_record_launch`, `_release_unlaunched` and `join_process`
//! make on `dict(row)`, with Python's comparisons. Capability-internal
//! helpers, not a use case and not a port.
use serde_json::Value;

use super::dto::{LaunchIdentity, MemberRow};
use crate::domain::swarm::{python_equal, status_is_alive};

/// `bounded(member, 'member', 128)`: the longest member id, in UTF-8 bytes.
pub(crate) const MEMBER_MAX_BYTES: usize = 128;

/// A member whose death is not confirmed, by the domain's one allowlist
/// ([`status_is_alive`]): the owner-decided divergence
/// `unknown_member_status_is_not_alive` (#2295) where Python refuses only
/// `'dead'`.
pub(crate) fn alive(row: &MemberRow) -> bool {
    status_is_alive(row.text("status"))
}

/// The member's row is `live`.
pub(crate) fn live(row: &MemberRow) -> bool {
    matches!(row.text("status"), Some("live"))
}

/// A `reserved` row with no process recorded: an admission never launched.
pub(crate) fn unlaunched(row: &MemberRow) -> bool {
    matches!(row.text("status"), Some("reserved")) && matches!(row.get("pid"), Some(Value::Null))
}

/// `row['reservation'] == reservation`, by Python's `==`: a NULL
/// reservation equals only `None`, and a stored `'5'` is not `5`.
pub(crate) fn holds_reservation(row: &MemberRow, reservation: &Value) -> bool {
    row.get("reservation")
        .is_some_and(|held| python_equal(held, reservation))
}

/// `row['pid'] == pid and row['started'] == started`, by Python's `==`:
/// the row names this process. A stored pid equals a number exactly
/// (`7 == 7.0`, `1 == True`, but a REAL 2^63 is not `i64::MAX`), never
/// its text; a NULL equals only `None`.
pub(crate) fn same_process(row: &MemberRow, launch: &LaunchIdentity) -> bool {
    let equal = |column: &str, given: &Value| {
        row.get(column)
            .is_some_and(|stored| python_equal(stored, given))
    };
    equal("pid", &launch.pid) && equal("started", &launch.started)
}

/// `row['pid'] is not None and (row['pid'], row['started']) != (pid, started)`:
/// a process is recorded and it is another.
pub(crate) fn launched_elsewhere(row: &MemberRow, launch: &LaunchIdentity) -> bool {
    match row.get("pid") {
        Some(Value::Null) | None => false,
        Some(_) => !same_process(row, launch),
    }
}

#[cfg(test)]
#[path = "board_membership_tests.rs"]
mod tests;
