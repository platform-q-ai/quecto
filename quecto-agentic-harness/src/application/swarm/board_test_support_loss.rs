//! Test support (compiled only under `cfg(test)`): the in-memory board's
//! loss reads (#2277): a member's launcher, the loss scan by Python's `==`
//! and the loss observations. A member id matches its row as TEXT
//! affinity roughly would; the SQLite adapter's contract tests pin the
//! real binding.
use serde_json::Value;

use super::{RecordedEvent, text_affinity};
use crate::application::swarm::dto::{MemberRow, ScopeObservation};
use crate::domain::swarm::python_equal;

/// `SELECT launcher FROM members WHERE id=?`.
pub(super) fn launcher(members: &[MemberRow], id: &Value) -> Option<Option<String>> {
    let id = text_affinity(id);
    members
        .iter()
        .find(|row| !id.is_null() && row.get("id") == Some(&id))
        .map(|row| row.text("launcher").map(str::to_owned))
}

/// `lost_after_activation(member)`: the latest `scope_unknown` naming
/// `member` newer than the latest `activated` naming it, by position.
pub(super) fn lost_after_activation(events: &[RecordedEvent], member: &Value) -> bool {
    let latest = |action: &str| {
        events
            .iter()
            .rposition(|event| {
                event.action == action
                    && event
                        .detail
                        .get("member")
                        .is_some_and(|named| python_equal(named, member))
            })
            .map(|index| index + 1)
    };
    latest("scope_unknown") > latest("activated")
}

/// Every `scope_observed` event, in order.
pub(super) fn scope_observations(events: &[RecordedEvent]) -> Vec<ScopeObservation> {
    events
        .iter()
        .filter(|event| event.action == "scope_observed")
        .map(|event| ScopeObservation {
            actor: Some(event.actor.clone()),
            time: Value::from(event.time),
            member: event.detail.get("member").cloned().unwrap_or(Value::Null),
        })
        .collect()
}
