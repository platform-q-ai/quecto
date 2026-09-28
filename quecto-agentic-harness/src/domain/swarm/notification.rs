//! Wake-notification policy ported from `swarm_policy.py` (#2267, #2127):
//! which live members an event batch wakes. Reads and acknowledgements never
//! wake a peer; ready work goes to the members free to take it.
use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::records::{MemberRecord, RunRecord, TaskState};

/// Events after which ready work may be waiting for a taker (#2127).
pub const READY_WORK_ACTIONS: [&str; 11] = [
    "task_created",
    "dependencies",
    "released",
    "verified",
    "revalidated",
    "recovered",
    "revoked",
    "claimed",
    "submitted",
    "blocked",
    "death_confirmed",
];
/// A worker's own progress: it concerns the coordinator only through its own
/// submitted/blocked wake, not as a taker of the remaining ready work.
pub const OWNERSHIP_ACTIONS: [&str; 3] = ["claimed", "submitted", "blocked"];
/// Task statuses in which the owner is working on, or waiting on, its own task.
pub const WORK_HOLDING_STATUSES: [&str; 3] = ["claimed", "blocked", "submitted"];
/// Of those, the ones in which the owner is still working (not parked).
pub const WORKING_STATUSES: [&str; 2] = ["claimed", "blocked"];

/// One board event as the wake policy reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct NotificationEvent {
    pub action: String,
    /// The event's JSON detail object; ids in it are integers.
    pub detail: Value,
    /// The member that caused the event, when the event records one.
    pub actor: Option<String>,
}

/// A task as the wake policy reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskSummary {
    pub id: i64,
    pub status: TaskState,
    pub dependencies: Vec<i64>,
    pub owner: Option<String>,
}

/// The board state an event batch is judged against.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationState {
    pub tasks: Vec<TaskSummary>,
    /// Ids of the messages still unread.
    pub unread: BTreeSet<i64>,
}

/// The live members (never `actor`) that `events` wake, sorted by id.
pub fn notification_targets(
    _run: &RunRecord,
    _actor: &str,
    _members: &[MemberRecord],
    _events: &[NotificationEvent],
    _state: &NotificationState,
) -> Vec<MemberRecord> {
    Vec::new()
}

/// Who an event hands ready work to (#2127), from the event actor's view.
pub fn ready_work_takers(
    _run: &RunRecord,
    _event_actor: &str,
    _everyone: &BTreeSet<String>,
    _tasks: &BTreeMap<i64, &TaskSummary>,
) -> BTreeSet<String> {
    BTreeSet::new()
}

#[cfg(test)]
#[path = "notification_tests.rs"]
mod notification_tests;
