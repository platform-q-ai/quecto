//! Wake-notification policy ported from `swarm_policy.py` (#2267, #2127):
//! which live members an event batch wakes. Reads and acknowledgements never
//! wake a peer; ready work goes to the members free to take it.
use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::BoardError;
use super::records::{MemberRecord, MemberState, RunRecord, RunState, TaskState};

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
    /// The event's JSON detail object. Ids in it are usually integers, but
    /// `submit`, `block` and `send` store the agent's value unchecked, so a
    /// float, a boolean or any other JSON value can appear.
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
///
/// Fails, as Python's `sorted(targets)` raises, when a message names a
/// recipient that is not a string beside a named target; a non-string
/// recipient alone wakes nobody.
pub fn notification_targets(
    run: &RunRecord,
    actor: &str,
    members: &[MemberRecord],
    events: &[NotificationEvent],
    state: &NotificationState,
) -> Result<Vec<MemberRecord>, BoardError> {
    // A NULL status (a row edited outside the board) is not running, as
    // Python's `None != 'running'`.
    match run.status.as_ref().map(RunState::as_str) {
        Some("running") => {}
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
    // Python type names of targets that are not member names (#2267 review):
    // `send` binds its recipient unchecked, so an event can name `5`.
    let mut strays: Vec<&'static str> = Vec::new();
    for event in events {
        let action = event.action.as_str();
        match action {
            "message_accepted" => {
                let unread = detail_id(event, "message", Lookup::SetElement)?
                    .is_some_and(|id| state.unread.contains(&id));
                // Python reads the recipient only for an unread message.
                match unread.then(|| detail(event, "recipient")).flatten() {
                    Some(Value::String(recipient)) => {
                        targets.insert(recipient.clone());
                    }
                    Some(stray @ (Value::Array(_) | Value::Object(_))) => {
                        return Err(unhashable(stray, Lookup::SetElement));
                    }
                    Some(stray @ (Value::Null | Value::Bool(_) | Value::Number(_))) => {
                        strays.push(python_type(stray));
                    }
                    None => {}
                }
            }
            "amended" => targets.extend(live.keys().map(|id| (*id).to_owned())),
            "evidence" | "death_confirmed" => {
                wake_coordinator(run, &mut targets, &mut strays);
            }
            "submitted" | "blocked" => {
                let task = detail_id(event, "task", Lookup::DictKey)?.and_then(|id| tasks.get(&id));
                if task.is_some_and(|task| task.status.as_str() == action) {
                    wake_coordinator(run, &mut targets, &mut strays);
                }
            }
            _ => {}
        }
        if ready && READY_WORK_ACTIONS.contains(&action) {
            let event_actor = event.actor.as_deref().unwrap_or(actor);
            let mut takers = ready_work_takers(run, event_actor, &everyone, &tasks);
            // Python's `takers - {None}` removes nobody.
            if let (true, Some(coordinator)) = (
                OWNERSHIP_ACTIONS.contains(&action),
                run.coordinator.as_ref(),
            ) {
                takers.remove(coordinator);
            }
            targets.extend(takers);
        }
    }
    if let Some(error) = sort_error(!targets.is_empty(), &strays) {
        return Err(BoardError::new(error));
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
    // Python's `free - {None}` is `free`: with no coordinator, anyone free
    // is a taker.
    if free
        .iter()
        .any(|identity| run.coordinator.as_deref() != Some(identity.as_str()))
    {
        return free;
    }
    let working = owners(tasks, &WORKING_STATUSES);
    others()
        .filter(|identity| !working.contains(identity.as_str()))
        .cloned()
        .collect()
}

/// Python's `m['status'] == 'live'`: a NULL status is not live.
fn is_live(member: &MemberRecord) -> bool {
    matches!(
        member.status.as_ref().map(MemberState::as_str),
        Some("live")
    )
}

/// Python's `targets.add(run['coordinator'])`. A NULL coordinator (a row
/// edited outside the board) adds `None`, which wakes nobody but, beside a
/// named target, makes `sorted(targets)` raise: it is recorded as a stray.
fn wake_coordinator(
    run: &RunRecord,
    targets: &mut BTreeSet<String>,
    strays: &mut Vec<&'static str>,
) {
    match &run.coordinator {
        Some(coordinator) => {
            targets.insert(coordinator.clone());
        }
        None => strays.push(python_type(&Value::Null)),
    }
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

/// A detail id as Python's dict and set lookups match it against the
/// board's integer ids: `true` is 1 and `false` 0, a float with no
/// fractional part in `i64` range is that integer (`1.0 == 1`, `-0.0 == 0`),
/// and a string or `null` matches no id. A list or an object cannot be
/// hashed: Python's lookup raises, and so does this.
fn detail_id(
    event: &NotificationEvent,
    key: &str,
    lookup: Lookup,
) -> Result<Option<i64>, BoardError> {
    match detail(event, key) {
        Some(Value::Bool(flag)) => Ok(Some(i64::from(*flag))),
        Some(Value::Number(number)) => Ok(number
            .as_i64()
            .or_else(|| number.as_f64().and_then(integral))),
        Some(value @ (Value::Array(_) | Value::Object(_))) => Err(unhashable(value, lookup)),
        Some(Value::Null | Value::String(_)) | None => Ok(None),
    }
}

/// Where Python hashes a detail value: `in unread` or `targets.add` (a set),
/// or `tasks.get` (a dict).
#[derive(Clone, Copy, Debug)]
enum Lookup {
    SetElement,
    DictKey,
}

/// The TypeError Python raises when it hashes a list or a dict. The text is
/// CPython 3.14's (the runtime the Python board runs on); earlier versions
/// say only `unhashable type: 'list'`.
fn unhashable(value: &Value, lookup: Lookup) -> BoardError {
    let kind = python_type(value);
    let role = match lookup {
        Lookup::SetElement => "a set element",
        Lookup::DictKey => "a dict key",
    };
    BoardError::new(format!(
        "cannot use '{kind}' as {role} (unhashable type: '{kind}')"
    ))
}

/// The integer a float equals, when it is whole and within `i64`.
fn integral(value: f64) -> Option<i64> {
    const BOUND: f64 = 9_223_372_036_854_775_808.0; // 2**63, exact in f64
    let whole = value.fract() == 0.0 && (-BOUND..BOUND).contains(&value);
    // The range check makes the cast exact; outside it the value is unused.
    whole.then_some(value as i64)
}

/// Python's `type(value).__name__` for a decoded JSON value. serde_json
/// reads `-0` and integers beyond `u64` as floats, so they name `float` where
/// Python's `json.loads` gives `int`. TODO(#2270): S5 decodes events with
/// S3's Python-compatible JSON codec (#2268), which keeps them integers.
fn python_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(number) if number.is_f64() => "float",
        Value::Number(_) => "int",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}

/// The TypeError Python's `sorted(targets)` raises when the target set mixes
/// kinds that do not order against each other: numbers (`int`, `bool`,
/// `float`), `None` and strings. Which pair Python names depends on the
/// set's iteration order, which its hash seed and insertion order decide:
/// with `None` in the set (hashed to a constant since CPython 3.12) it
/// usually iterates first and Python compares `str < None`, but a name
/// inserted first can take its slot, and then Python compares `None < str`
/// (34 of 200 seeds when a message to a name precedes the `None`). The
/// board fixes one text per case: `str`/`NoneType` beside a name,
/// `number`/`NoneType` beside a number, the stray number against a string
/// without `None`. The error is certain; only its operand order can differ
/// from a given Python process, and only for values the board never writes
/// (a NULL coordinator or a non-string recipient in an edited file).
fn sort_error(named: bool, strays: &[&'static str]) -> Option<String> {
    let numeric = strays
        .iter()
        .copied()
        .find(|kind| matches!(*kind, "int" | "bool" | "float"));
    let none = strays.contains(&"NoneType");
    let (left, right) = match (none, named, numeric) {
        (true, true, _) => ("str", "NoneType"),
        (true, false, Some(number)) => (number, "NoneType"),
        (false, true, Some(number)) => (number, "str"),
        (true, false, None) | (false, _, _) => return None,
    };
    Some(format!(
        "'<' not supported between instances of '{left}' and '{right}'"
    ))
}

#[cfg(test)]
#[path = "notification_tests.rs"]
mod notification_tests;

#[cfg(test)]
#[path = "notification_python_types_tests.rs"]
mod notification_python_types_tests;
