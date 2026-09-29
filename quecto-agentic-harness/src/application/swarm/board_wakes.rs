//! What `_notifications` and `_accept_wake` share (#2276): the members an
//! event batch wakes, judged by the domain policy
//! ([`notification_targets`]) against the board as the transaction reads
//! it. A capability-internal helper, not a use case and not a port.
use super::dto::MemberRow;
use super::ports::{BoardMembers, BoardWakes};
use crate::domain::swarm::{
    BoardError, MemberRecord, MemberState, NotificationEvent, RunRecord, RunState,
    notification_targets,
};

/// Whether the run is running (Python's `run['status'] == 'running'`).
pub(crate) fn running(run: &RunRecord) -> bool {
    matches!(run.status.as_ref().map(RunState::as_str), Some("running"))
}

/// The member a `members` row is to the policy: its text id, status and
/// reservation. A row whose id is not text (only a hand edit writes one)
/// is no member the policy can name, so it is never a target. Python does
/// find such a row: a free member with a NULL id is one of its targets, and
/// `sorted(targets)` then raises `TypeError` beside a named one, where the
/// Rust board answers the named ones (the `outside_edited_wake_records`
/// divergence, pinned in `tests/integration/swarm_board_diff_loose_messages.rs`).
fn record(row: &MemberRow) -> Option<MemberRecord> {
    Some(MemberRecord {
        id: row.text("id")?.to_owned(),
        status: row.text("status").map(MemberState::new),
        reservation: row.text("reservation").map(str::to_owned),
    })
}

/// `notification_targets(run, actor, tx.members(), events,
/// tx.notification_state())`: the rows of the live members `events` wake,
/// as `dict(row)`, sorted by id. The members and the state are read in
/// Python's order, events or not.
///
/// # Errors
/// The store's, or the policy's `TypeError` when the targets do not sort.
pub(crate) fn woken(
    transaction: &(impl BoardMembers + BoardWakes + ?Sized),
    run: &RunRecord,
    actor: &str,
    events: &[NotificationEvent],
) -> Result<Vec<MemberRow>, BoardError> {
    let rows = transaction.members()?;
    let state = transaction.notification_state()?;
    let members: Vec<MemberRecord> = rows.iter().filter_map(record).collect();
    let targets = notification_targets(run, actor, &members, events, &state)?;
    let found: Vec<MemberRow> = targets
        .iter()
        .filter_map(|target| {
            rows.iter()
                .find(|row| row.text("id") == Some(target.id.as_str()))
                .cloned()
        })
        .collect();
    debug_assert_eq!(found.len(), targets.len(), "every target is a member row");
    Ok(found)
}

#[cfg(test)]
#[path = "board_wakes_tests.rs"]
mod tests;
