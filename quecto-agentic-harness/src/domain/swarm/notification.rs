//! Wake-notification policy ported from `swarm_policy.py` (#2267, #2127):
//! which live members an event batch wakes. Reads and acknowledgements never
//! wake a peer; ready work goes to the members free to take it.
use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::BoardError;
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

/// The live members that `events` wake, sorted by id: never `actor`, and
/// only while the run is running. Each event is judged from its own actor's
/// view (`event.actor`, else `actor`), so the sender and a receiver's
/// re-check agree. A message wakes its recipient while it is unread; an
/// amended contract wakes everyone; evidence and a confirmed death wake the
/// coordinator, as does a submitted or blocked task still in that status.
/// Whenever ready work exists, a ready-work event wakes its takers.
pub fn notification_targets(
    run: &RunRecord,
    actor: &str,
    members: &[MemberRecord],
    events: &[NotificationEvent],
    state: &NotificationState,
) -> Result<Vec<MemberRecord>, BoardError> {
    match run.status.as_str() {
        "running" => {}
        _ => return Ok(Vec::new()),
    }
    // Python's dict comprehensions: a later duplicate id wins.
    let tasks: BTreeMap<i64, &TaskSummary> =
        state.tasks.iter().map(|task| (task.id, task)).collect();
    let ready = tasks.values().any(|task| ready_to_take(task, &tasks));
    let live: BTreeMap<&str, &MemberRecord> = members
        .iter()
        .filter(|member| is_live(member) && member.id != actor)
        .map(|member| (member.id.as_str(), member))
        .collect();
    let everyone: BTreeSet<String> = members
        .iter()
        .filter(|member| is_live(member))
        .map(|member| member.id.clone())
        .collect();
    let mut targets = BTreeSet::new();
    for event in events {
        let action = event.action.as_str();
        match action {
            "message_accepted" => {
                let unread =
                    detail_id(event, "message").is_some_and(|id| state.unread.contains(&id));
                let recipient = detail(event, "recipient").and_then(Value::as_str);
                if let (true, Some(recipient)) = (unread, recipient) {
                    targets.insert(recipient.to_owned());
                }
            }
            "amended" => targets.extend(live.keys().map(|id| (*id).to_owned())),
            "evidence" | "death_confirmed" => {
                targets.insert(run.coordinator.clone());
            }
            "submitted" | "blocked" => {
                let task = detail_id(event, "task").and_then(|id| tasks.get(&id));
                if task.is_some_and(|task| task.status.as_str() == action) {
                    targets.insert(run.coordinator.clone());
                }
            }
            _ => {}
        }
        if ready && READY_WORK_ACTIONS.contains(&action) {
            let event_actor = event.actor.as_deref().unwrap_or(actor);
            let mut takers = ready_work_takers(run, event_actor, &everyone, &tasks);
            if OWNERSHIP_ACTIONS.contains(&action) {
                takers.remove(&run.coordinator);
            }
            targets.extend(takers);
        }
    }
    let woken: Vec<MemberRecord> = targets
        .iter()
        .filter_map(|identity| live.get(identity.as_str()).map(|member| (*member).clone()))
        .collect();
    debug_assert!(
        woken
            .iter()
            .all(|member| member.id != actor && is_live(member)),
        "only live members other than the actor are woken"
    );
    Ok(woken)
}

/// Who an event hands ready work to (#2127), from the event actor's view.
///
/// A member holding a claimed, blocked or submitted task has its work, so
/// ready work is for the others. When none of them but the coordinator (which
/// never claims) is free, members that only wait for review (parked) take it
/// after all, so it is never left idle. The event's actor is never a taker.
/// As in Python, a free set holding anyone but the coordinator is returned
/// whole, the coordinator included.
pub fn ready_work_takers(
    run: &RunRecord,
    event_actor: &str,
    everyone: &BTreeSet<String>,
    tasks: &BTreeMap<i64, &TaskSummary>,
) -> BTreeSet<String> {
    let others = || {
        everyone
            .iter()
            .filter(|identity| identity.as_str() != event_actor)
    };
    let holding = owners(tasks, &WORK_HOLDING_STATUSES);
    let free: BTreeSet<String> = others()
        .filter(|identity| !holding.contains(identity.as_str()))
        .cloned()
        .collect();
    if free.iter().any(|identity| *identity != run.coordinator) {
        return free;
    }
    let working = owners(tasks, &WORKING_STATUSES);
    others()
        .filter(|identity| !working.contains(identity.as_str()))
        .cloned()
        .collect()
}

fn is_live(member: &MemberRecord) -> bool {
    member.status.as_str() == "live"
}

/// A ready task whose every dependency the board holds as completed.
fn ready_to_take(task: &TaskSummary, tasks: &BTreeMap<i64, &TaskSummary>) -> bool {
    task.status.as_str() == "ready"
        && task.dependencies.iter().all(|dependency| {
            tasks
                .get(dependency)
                .is_some_and(|dependency| dependency.status.as_str() == "completed")
        })
}

/// The owners (a nonempty name) of the tasks in one of `statuses`.
fn owners<'a>(tasks: &BTreeMap<i64, &'a TaskSummary>, statuses: &[&str]) -> BTreeSet<&'a str> {
    tasks
        .values()
        .filter(|task| statuses.contains(&task.status.as_str()))
        .filter_map(|task| task.owner.as_deref().filter(|owner| !owner.is_empty()))
        .collect()
}

/// A key the board always writes into this action's detail. Python raises
/// `KeyError` without it; here a missing key matches nothing in release.
fn detail<'a>(event: &'a NotificationEvent, key: &str) -> Option<&'a Value> {
    let value = event.detail.get(key);
    debug_assert!(
        value.is_some(),
        "a {} event carries its key {key}",
        event.action
    );
    value
}

/// An integer id from the detail; any other JSON value matches no id.
fn detail_id(event: &NotificationEvent, key: &str) -> Option<i64> {
    detail(event, key).and_then(Value::as_i64)
}

#[cfg(test)]
#[path = "notification_tests.rs"]
mod notification_tests;
