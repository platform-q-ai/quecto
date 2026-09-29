//! What the read models share (#2277, #1969): the port of
//! `Workbench.summary`, `_liveness_watch`, `Tasks._with_owner_liveness`
//! and `_owner_views`, and the page checks. Capability-internal helpers,
//! not a use case and not a port: `ReadRunSummary`, `CreateRun`,
//! `JoinMember` and `BootstrapMember` build the summary here, `ReadTask`
//! and `ListTasks` the owner liveness.
//!
//! A task owner the board never writes (not text) reads as `unknown`
//! with no activity; an event time that is not a number, where the
//! liveness measures from it, is refused naming the record; a task whose
//! status `summary` does not count is refused the same way. Python
//! raises on each (the `outside_edited_task_columns` and
//! `outside_edited_loss_records` divergences).
use serde_json::Value;

use super::board_control::edited;
use super::board_operation::{atomic, operation};
use super::board_tasks::{HELD_CLAIM, read_task};
use super::dto::{CountedTask, FullSummary, RunSummary, SummaryCounts, SummaryScan, TaskRow};
use super::ports::{
    BoardEvents, BoardMembers, BoardRepository, BoardTasks, BoardTransaction, Clock,
};
use crate::domain::swarm::{
    Access, BoardError, OWNER_IDLE_AFTER, OwnerState, PythonLookup, RefusalKind, authorize,
    idle_transition, owner_recovery, owner_state, python_repr,
};

/// The tasks and files a summary holds.
const SUMMARY_PAGE: i64 = 50;
/// The largest page a member may ask for.
const PAGE_MAX: i64 = 100;

/// Python's `type(offset) is int and offset >= 0` and `type(limit) is
/// int and 1 <= limit <= 100`: a JSON integer, never a boolean, a float or
/// text.
pub(crate) fn page_bounds(offset: &Value, limit: &Value) -> Option<(u64, i64)> {
    let limit = limit
        .as_i64()
        .filter(|limit| (1..=PAGE_MAX).contains(limit));
    Some((offset.as_u64()?, limit?))
}

/// The read-only gate's access (`active=False, read_only=True`).
pub(crate) fn reading() -> Access {
    Access {
        read_only: true,
        ..Access::default()
    }
}

/// `max(0.0, now - last)`: Python keeps its first argument unless the
/// second is greater.
fn since_activity(now: f64, last: f64) -> Value {
    let elapsed = now - last;
    Value::from(if elapsed > 0.0 { elapsed } else { 0.0 })
}

/// `Tasks._owner_views(db, owned, now)`: `(last_activity, owner_state)`
/// per owner given (one per task), from three bounded reads: the owners'
/// member rows, one grouped scan of their events and one loss scan.
fn owner_views(
    transaction: &(impl BoardMembers + BoardEvents + ?Sized),
    owners: &[Value],
    now: f64,
) -> Result<Vec<(Option<f64>, OwnerState)>, BoardError> {
    let mut names: Vec<&str> = owners.iter().filter_map(Value::as_str).collect();
    names.sort_unstable();
    names.dedup();
    let statuses = transaction.member_statuses(&names)?;
    let latest = transaction.latest_activity(&names)?;
    let lost = transaction.lost_members(&names)?;
    owners
        .iter()
        .map(|owner| {
            let Some(name) = owner.as_str() else {
                return Ok((None, OwnerState::Unknown));
            };
            let status = statuses
                .iter()
                .find(|(id, _)| id == name)
                .and_then(|(_, status)| status.as_deref());
            let last = match latest.iter().find(|activity| activity.actor == name) {
                None => None,
                Some(activity) if activity.latest.is_null() => None,
                Some(activity) => Some(
                    activity
                        .latest
                        .as_f64()
                        .ok_or_else(|| edited("event time"))?,
                ),
            };
            let lost = lost.iter().any(|member| member == name);
            Ok((last, owner_state(status, lost, last, now, OWNER_IDLE_AFTER)))
        })
        .collect()
}

/// Whether a task holds a claim with an owner (`Tasks.owned_tasks`).
fn owned(task: &TaskRow) -> bool {
    let held = task
        .text("status")
        .is_some_and(|status| HELD_CLAIM.contains(&status));
    held && task.get("owner").is_some_and(|owner| !owner.is_null())
}

/// `Tasks._with_owner_liveness(db, tasks)`: a claimed, blocked or
/// submitted task carries its owner's `owner_last_activity` (seconds since
/// the owner's latest board event, by the reader's clock), `owner_state`,
/// and `contact` (how `send` reaches an owner it accepts) or, for any
/// other owner, `contact` `null` and `recovery`. Unowned tasks carry none
/// of these. Nothing is written. How many owners it read (one per owned
/// task), for telemetry.
///
/// # Errors
/// An event time that is not a number, or the store's.
pub(crate) fn with_owner_liveness(
    transaction: &(impl BoardMembers + BoardEvents + ?Sized),
    clock: &dyn Clock,
    tasks: &mut [TaskRow],
) -> Result<u64, BoardError> {
    let held: Vec<usize> = (0..tasks.len())
        .filter(|&index| owned(&tasks[index]))
        .collect();
    if held.is_empty() {
        return Ok(0);
    }
    let now = clock.now_seconds();
    let owners: Vec<Value> = held
        .iter()
        .map(|&index| tasks[index].get("owner").cloned().unwrap_or(Value::Null))
        .collect();
    let views = owner_views(transaction, &owners, now)?;
    debug_assert_eq!(views.len(), held.len(), "one view per owned task");
    for ((&index, owner), (last, state)) in held.iter().zip(&owners).zip(views) {
        let task = &mut tasks[index];
        let activity = last.map_or(Value::Null, |last| since_activity(now, last));
        task.set("owner_last_activity", activity);
        task.set("owner_state", Value::from(state.as_str()));
        if state.addressable() {
            let name = owner
                .as_str()
                .map_or_else(|| owner.to_string(), python_repr);
            task.set(
                "contact",
                Value::from(format!("board.send(request, {name}, body)")),
            );
        } else {
            task.set("contact", Value::Null);
            task.set("recovery", Value::from(owner_recovery(state)));
        }
    }
    Ok(count(held.len()))
}

/// A length as a telemetry count.
fn count(length: usize) -> u64 {
    u64::try_from(length).unwrap_or(u64::MAX)
}

/// What [`liveness_watch`] found.
struct Watch {
    /// When the earliest still-active owner turns idle.
    next: Option<f64>,
    /// Whether an owner turned idle by the clock alone since the cursor.
    crossed: bool,
    /// The owners it read.
    scanned: u64,
}

/// `Workbench._liveness_watch(db, cursor)`: when the earliest still-active
/// owner turns idle (`None` when none will), and whether an owner is idle
/// now but was not at the time of event `cursor` (it turned idle by the
/// clock alone, so no event moved the cursor), with how many owners it
/// read. Nothing is written.
fn liveness_watch(
    transaction: &(impl BoardMembers + BoardEvents + BoardTasks + ?Sized),
    clock: &dyn Clock,
    cursor: i64,
) -> Result<Watch, BoardError> {
    let owners = transaction.claim_owners()?;
    if owners.is_empty() {
        return Ok(Watch {
            next: None,
            crossed: false,
            scanned: 0,
        });
    }
    let now = clock.now_seconds();
    let cursor_time = transaction.event_time(cursor)?.unwrap_or(Value::from(0.0));
    let (mut next, mut crossed) = (None::<f64>, false);
    for (last, state) in owner_views(transaction, &owners, now)? {
        match (state, last) {
            (OwnerState::Idle, Some(last)) => {
                let then = cursor_time.as_f64().ok_or_else(|| edited("event time"))?;
                crossed |= last + OWNER_IDLE_AFTER > then;
            }
            (OwnerState::Active, last) => {
                // An active owner has `now - last` below the threshold, so
                // it turns idle after `now`. The subtraction is exact when
                // Sterbenz's condition holds (`last / 2 <= now <= 2 *
                // last`), as it does for real clocks, so `last +
                // OWNER_IDLE_AFTER` exceeds `now` as a real number, and
                // rounding it cannot fall below `now`. It lands on `now`
                // only in a half-ulp tie rounded down, which needs `last`
                // below a binade boundary and `now` at or above it (for
                // Unix times, the 2^31 s crossing in 2038, where the ulp
                // doubles): vanishingly rare. There, or with times an edit
                // wrote outside Sterbenz's condition, `None` is left, on
                // which Python's `min` would raise; that owner is passed
                // over here.
                if let Some(at) = idle_transition(last, now, OWNER_IDLE_AFTER) {
                    next = Some(next.map_or(at, |earlier| earlier.min(at)));
                }
            }
            _ => {}
        }
    }
    Ok(Watch {
        next,
        crossed,
        scanned: count(owners.len()),
    })
}

/// `Workbench.summary(since)` as `actor`, through the read-only gate: the
/// fast path `unchanged` when `since` is the board's cursor and no owner
/// turned idle since, the whole summary otherwise. `None` is a Python
/// `Workbench` whose member is `None` (a coordinator that is nobody): the
/// gate refuses it.
///
/// # Errors
/// An authorisation refusal, an edited record, or the store's.
pub(crate) fn summary(
    repository: &dyn BoardRepository,
    clock: &dyn Clock,
    actor: Option<&str>,
    since: Option<i64>,
) -> Result<RunSummary, BoardError> {
    let Some(actor) = actor else {
        return atomic(repository, false, |transaction| {
            authorize(transaction.run()?.as_ref(), "", None, reading())?;
            Err(BoardError::new(
                RefusalKind::Internal,
                "the gate admitted a member that is nobody",
            ))
        });
    };
    operation(repository, clock, actor, reading(), |transaction, run| {
        let cursor = transaction.event_generation()?;
        let Watch {
            next,
            crossed,
            scanned,
        } = liveness_watch(transaction, clock, cursor)?;
        let row = transaction
            .run_row()?
            .ok_or_else(|| BoardError::new(RefusalKind::RunMissing, "coordination run missing"))?;
        let current = since == Some(cursor);
        let mut scan = SummaryScan {
            owners_scanned: scanned,
            fast_path_defeated: current.then_some(crossed),
            cursor_moved: since.map(|_| !current),
        };
        if current && !crossed {
            return Ok(RunSummary::Unchanged {
                event_cursor: cursor,
                status: row.get("status").cloned().unwrap_or(Value::Null),
                next_liveness_check_at: next,
                scan,
            });
        }
        let members = transaction.members()?;
        let usage = members
            .iter()
            .filter(|member| matches!(member.text("status"), Some("live" | "reserved")))
            .count();
        let mut tasks = transaction
            .task_ids(0, SUMMARY_PAGE)?
            .iter()
            .map(|id| read_task(transaction, id))
            .collect::<Result<Vec<_>, _>>()?;
        let page_owners = with_owner_liveness(transaction, clock, &mut tasks)?;
        scan.owners_scanned = scan.owners_scanned.saturating_add(page_owners);
        Ok(RunSummary::Full(Box::new(FullSummary {
            run: row,
            next_liveness_check_at: next,
            members,
            usage: i64::try_from(usage).unwrap_or(i64::MAX),
            task_count: transaction.task_count()?,
            tasks,
            file_count: transaction.file_count()?,
            files: transaction.file_page(0, SUMMARY_PAGE)?,
            evidence: transaction.evidence_rows()?,
            control_generation: transaction.control_generation()?,
            event_cursor: cursor,
            counts: counts(transaction, run.coordinator.as_deref())?,
            scan,
        })))
    })
}

/// `summary()['counts']`: each task by its status, a `ready` one with a
/// dependency that is not `completed` (or names no task) counted
/// `blocked`, then `member_claim_counts`.
fn counts(
    transaction: &(impl BoardTransaction + ?Sized),
    coordinator: Option<&str>,
) -> Result<SummaryCounts, BoardError> {
    let states = transaction.task_states()?;
    // Python's `states[d]` is a dict lookup: by hash, not a search.
    let ids = PythonLookup::new(states.iter().map(|task| &task.id));
    let completed = |dependency: &Value| {
        ids.position(dependency)
            .is_some_and(|index| states[index].status.as_str() == Some("completed"))
    };
    let mut counts = SummaryCounts {
        members: transaction.claim_counts(coordinator)?,
        ..SummaryCounts::default()
    };
    for task in &states {
        let unmet = || {
            dependencies(task)
                .iter()
                .any(|dependency| !completed(dependency))
        };
        let slot = match task.status.as_str() {
            Some("ready") if unmet() => &mut counts.blocked,
            Some("ready") => &mut counts.ready,
            Some("claimed") => &mut counts.claimed,
            Some("blocked") => &mut counts.blocked,
            Some("submitted") => &mut counts.submitted,
            Some("completed") => &mut counts.completed,
            _ => return Err(edited("task status")),
        };
        *slot += 1;
    }
    Ok(counts)
}

/// A task's stored dependencies, read as `TaskRow::dependencies` reads
/// them: the list, or none.
fn dependencies(task: &CountedTask) -> &[Value] {
    task.dependencies.as_array().map_or(&[], Vec::as_slice)
}

#[cfg(test)]
#[path = "board_read_models_tests.rs"]
mod tests;
