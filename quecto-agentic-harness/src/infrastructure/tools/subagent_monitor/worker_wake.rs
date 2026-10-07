//! A coordinator's swarm workers report through its board (#2471). Their
//! submissions, blocks, messages and deaths wake the coordinator there, so a
//! worker's plain turn end reaches it only as:
//! - stranded work, read from the board: a task claimed by a worker that is
//!   not working, or ready work while no worker works. Reported once, and
//!   again when it grows or a worker it names takes another turn;
//! - the first good turn after the worker failed (its error is still
//!   remembered by the coordinator's note queue).
//!
//! A reply to the coordinator's own `prompt`, `steer` or `follow_up` comes
//! as the worker's `reply_ready`, from its idle boundary. An unreadable
//! board, or one that does not name this process coordinator, sends the
//! ordinary note. Only a worker launched by its container's creator
//! ([`launcher_board_for`]) takes part; every other child keeps the
//! ordinary note on every turn end.

use std::collections::BTreeSet;

use super::super::subagent_registry::{
    NotificationTx, SubagentEntry, SubagentNotification, SubagentRegistry, SubagentStatus,
    SwarmNoteState,
};
use crate::application::swarm::ports::WorkerBoardRead;
use crate::domain::swarm::worker_wake::{Stranded, stranded_work};

/// A worker's part in its coordinator's wakes, on its registry entry.
#[derive(Debug, Clone, Default)]
pub struct WorkerWake {
    /// The board its plain turn ends are settled by: set only for a swarm
    /// worker launched by its container's creator.
    pub launcher_board: Option<LauncherBoard>,
    /// Its last turn failed: the next good turn end is sent as it was.
    after_failure: bool,
    /// The stranded work last reported for this coordinator's workers
    /// (kept on every worker's entry alike).
    stall_reported: Option<Stranded>,
}

/// The board a worker's turn ends are read against, and the worker's member
/// id on it.
#[derive(Clone)]
pub struct LauncherBoard {
    pub read: std::sync::Arc<dyn WorkerBoardRead>,
    pub member: String,
}

impl std::fmt::Debug for LauncherBoard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LauncherBoard({})", self.member)
    }
}

impl WorkerWake {
    /// Its next good turn end is sent as it was.
    pub(super) fn turn_failed(&mut self) {
        self.after_failure = true;
    }
}

/// The board a newly launched child's turn ends are settled by (#2471):
/// only a swarm worker (it holds a board `member` reservation) launched by
/// the process that created its container, and so its run.
pub fn launcher_board_for(
    created_run: bool,
    member: Option<String>,
    read: Option<std::sync::Arc<dyn WorkerBoardRead>>,
) -> Option<LauncherBoard> {
    match (created_run, member, read) {
        (true, Some(member), Some(read)) => Some(LauncherBoard { read, member }),
        (true, _, _) | (false, _, _) => None,
    }
}

/// A worker's `reply_ready`: the reply to the coordinator's own instruction
/// is ready, so it gets the ordinary note. `false` for any other child.
pub(super) fn reply_ready(
    registry: &SubagentRegistry,
    notify_tx: Option<&NotificationTx>,
    agent_id: &str,
    sequence: u64,
) -> bool {
    let worker = registry
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(agent_id)
        .is_some_and(|entry| entry.coordinator_wake.worker.launcher_board.is_some());
    if worker {
        let note = Box::new(|agent_id| SubagentNotification::Completed { agent_id });
        super::swarm_wake::send(registry, notify_tx, agent_id, sequence, note);
    }
    worker
}

/// A worker starting a turn re-arms a stall report that names it (or that
/// counted ready work while none worked).
pub(super) fn worker_started(registry: &SubagentRegistry, agent_id: &str) {
    let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    let member = entries
        .get(agent_id)
        .and_then(|entry| entry.coordinator_wake.worker.launcher_board.as_ref())
        .map(|board| board.member.clone());
    let Some(member) = member else {
        return;
    };
    let rearmed = workers(&entries).any(|entry| {
        entry
            .coordinator_wake
            .worker
            .stall_reported
            .as_ref()
            .is_some_and(|reported| reported.rearmed_by(&member))
    });
    if rearmed {
        record_stall(entries.values_mut(), None);
    }
}

/// Settle a swarm worker's plain turn end at its coordinator: `true` when
/// it is handled here (no note, or stranded work reported once the board is
/// read), `false` to send the ordinary note.
pub(super) fn worker_turn_end(
    registry: &SubagentRegistry,
    notify_tx: Option<&NotificationTx>,
    agent_id: &str,
) -> bool {
    let board = {
        let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
        let Some(entry) = entries.get_mut(agent_id) else {
            return false;
        };
        let Some(board) = entry.coordinator_wake.worker.launcher_board.clone() else {
            return false;
        };
        debug_assert!(
            matches!(entry.status, SubagentStatus::Idle | SubagentStatus::Exited),
            "a turn end is applied before it is settled"
        );
        match std::mem::take(&mut entry.coordinator_wake.worker.after_failure) {
            true => return false,
            false => board,
        }
    };
    let (Some(tx), Ok(runtime)) = (notify_tx.cloned(), tokio::runtime::Handle::try_current())
    else {
        return false;
    };
    let (registry, agent_id) = (registry.clone(), agent_id.to_owned());
    runtime.spawn(async move {
        let read = board.read.worker_board().await;
        let note: Box<dyn FnOnce(String) -> SubagentNotification + Send> = match read {
            Ok(Some(work)) => match newly_stranded(&registry, &work) {
                Some(Stranded { claimed_by, ready }) => {
                    let claimed = i64::try_from(claimed_by.len()).unwrap_or(i64::MAX);
                    Box::new(move |agent_id| SubagentNotification::SwarmState {
                        agent_id,
                        state: SwarmNoteState::WorkersIdle { claimed, ready },
                    })
                }
                None => return,
            },
            // Not the run's coordinator after all, or no board: the ordinary
            // note, unless a newer turn of this worker has begun or failed.
            Ok(None) | Err(_) => match still_idle(&registry, &agent_id) {
                true => Box::new(|agent_id| SubagentNotification::Completed { agent_id }),
                false => return,
            },
        };
        let sequence = {
            let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
            super::super::subagent_monitor_registry::next_sequence(&mut entries, &agent_id)
        };
        if let Some(sequence) = sequence {
            super::swarm_wake::send(&registry, Some(&tx), &agent_id, sequence, note);
        }
    });
    true
}

fn still_idle(registry: &SubagentRegistry, agent_id: &str) -> bool {
    registry
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(agent_id)
        .is_some_and(|entry| matches!(entry.status, SubagentStatus::Idle))
}

fn workers(
    entries: &std::collections::HashMap<String, SubagentEntry>,
) -> impl Iterator<Item = &SubagentEntry> {
    entries
        .values()
        .filter(|entry| entry.coordinator_wake.worker.launcher_board.is_some())
}

/// The stranded work `work` shows against the workers working now, when it
/// is news after the last report; the report is recorded on every worker.
fn newly_stranded(
    registry: &SubagentRegistry,
    work: &crate::domain::swarm::worker_wake::WorkerBoard,
) -> Option<Stranded> {
    let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    let working: BTreeSet<String> = workers(&entries)
        .filter(|entry| entry.status.is_active())
        .filter_map(|entry| {
            let board = entry.coordinator_wake.worker.launcher_board.as_ref()?;
            Some(board.member.clone())
        })
        .collect();
    let Some(stranded) = stranded_work(work, &working) else {
        record_stall(entries.values_mut(), None);
        return None;
    };
    let covered = workers(&entries).any(|entry| {
        entry
            .coordinator_wake
            .worker
            .stall_reported
            .as_ref()
            .is_some_and(|reported| reported.covers(&stranded))
    });
    match covered {
        true => None,
        false => {
            record_stall(entries.values_mut(), Some(stranded.clone()));
            Some(stranded)
        }
    }
}

fn record_stall<'a>(
    entries: impl Iterator<Item = &'a mut SubagentEntry>,
    report: Option<Stranded>,
) {
    for entry in entries {
        let worker = &mut entry.coordinator_wake.worker;
        if worker.launcher_board.is_some() {
            worker.stall_reported = report.clone();
        }
    }
}
