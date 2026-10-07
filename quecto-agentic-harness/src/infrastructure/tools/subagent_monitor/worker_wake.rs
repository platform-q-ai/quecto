//! A coordinator's swarm workers report through its board (#2471). Their
//! submissions, blocks, messages and deaths wake the coordinator there, so a
//! worker's plain turn end reaches it only as:
//! - the reply to an instruction the coordinator sent that turn;
//! - the first good turn after the worker failed (its error is still
//!   remembered by the coordinator's note queue);
//! - stranded work, read from the board: a task claimed by a worker that is
//!   not working, or ready work while no worker works. The same stranded
//!   work is reported once.
//!
//! An unreadable board, or one that does not name this process coordinator,
//! sends the ordinary note. Only a worker launched by its container's
//! creator ([`launcher_board_for`]) takes part; every other child keeps the
//! ordinary note on every turn end.

use std::collections::BTreeSet;

use super::super::subagent_registry::{
    NotificationTx, SubagentEntry, SubagentNotification, SubagentRegistry, SubagentStatus,
    SwarmNoteState,
};
use crate::application::swarm::ports::WorkerBoardRead;
use crate::domain::swarm::worker_wake::{
    Stranded, WorkerTurnEnd, settle_worker_turn_end, stranded_work,
};

/// A worker's part in its coordinator's wakes, on its registry entry.
#[derive(Debug, Clone, Default)]
pub struct WorkerWake {
    /// The board its plain turn ends are settled by: set only for a swarm
    /// worker launched by its container's creator.
    pub launcher_board: Option<LauncherBoard>,
    reply: Reply,
    /// Its last turn failed: the next good turn end is sent as it was.
    after_failure: bool,
    /// The stranded work last reported for this coordinator's workers
    /// (kept on every worker's entry alike).
    stall_reported: Option<String>,
}

/// An instruction the coordinator sent the worker.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Reply {
    #[default]
    None,
    /// Accepted, not yet running: the turn now ending is not its reply.
    Requested,
    /// Running in the worker's current turn: its end is the reply.
    InTurn,
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
    /// A new turn runs the instruction the coordinator sent, if any.
    pub(super) fn turn_started(&mut self) {
        if self.reply == Reply::Requested {
            self.reply = Reply::InTurn;
        }
    }

    /// A failed turn's error note answers any instruction it ran.
    pub(super) fn turn_failed(&mut self) {
        self.reply = Reply::None;
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

/// Send `command` to `agent_id` through `send`, recording that the
/// coordinator is owed its reply when it is an instruction the worker will
/// run (#2471). The debt is recorded before sending, so the turn cannot end
/// first, and withdrawn when the worker refuses it or the send fails.
pub async fn owing_reply<E>(
    registry: &SubagentRegistry,
    agent_id: &str,
    command: &str,
    send: impl std::future::Future<Output = Result<String, E>>,
) -> Result<String, E> {
    let expected = expect_reply(registry, agent_id, command);
    let sent = send.await;
    let accepted = sent.as_ref().is_ok_and(|response| {
        serde_json::from_str::<serde_json::Value>(response)
            .is_ok_and(|value| value["success"] != false)
    });
    if let (true, false) = (expected, accepted) {
        withdraw_reply(registry, agent_id);
    }
    sent
}

/// Record that a worker is about to be sent `command`; `true` when it owes
/// a reply. A prompt to a worker mid-turn is refused by the worker
/// (`streamingBehavior` is required), so it owes nothing.
pub(super) fn expect_reply(registry: &SubagentRegistry, agent_id: &str, command: &str) -> bool {
    let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    let key = super::super::subagent_registry::resolve_registry_key(&entries, agent_id);
    let Some(entry) = key.ok().and_then(|key| entries.get_mut(&key)) else {
        return false;
    };
    let busy = matches!(
        entry.status,
        SubagentStatus::Starting | SubagentStatus::Running
    );
    let worker = entry.coordinator_wake.worker.launcher_board.is_some();
    match (worker, command, busy) {
        (true, "steer" | "follow_up", _) | (true, "prompt", false) => {
            entry.coordinator_wake.worker.reply = Reply::Requested;
            true
        }
        (true, _, _) | (false, _, _) => false,
    }
}

fn withdraw_reply(registry: &SubagentRegistry, agent_id: &str) {
    let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    let key = super::super::subagent_registry::resolve_registry_key(&entries, agent_id);
    if let Some(entry) = key.ok().and_then(|key| entries.get_mut(&key)) {
        entry.coordinator_wake.worker.reply = Reply::None;
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
        debug_assert!(
            !matches!(
                entry.status,
                SubagentStatus::Starting | SubagentStatus::Running
            ),
            "a turn end is applied before it is settled"
        );
        let worker = &mut entry.coordinator_wake.worker;
        let Some(board) = worker.launcher_board.clone() else {
            return false;
        };
        let reply_due = worker.reply == Reply::InTurn;
        if reply_due {
            worker.reply = Reply::None;
        }
        let after_failure = std::mem::take(&mut worker.after_failure);
        match settle_worker_turn_end(reply_due, after_failure) {
            WorkerTurnEnd::Ordinary => return false,
            WorkerTurnEnd::CheckBoard => board,
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
            // Not the run's coordinator after all, or no board: as before.
            Ok(None) | Err(_) => Box::new(|agent_id| SubagentNotification::Completed { agent_id }),
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

/// The stranded work `work` shows against the workers working now, if it
/// was not already reported; the report is recorded on every worker.
fn newly_stranded(
    registry: &SubagentRegistry,
    work: &crate::domain::swarm::worker_wake::WorkerBoard,
) -> Option<Stranded> {
    let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    let workers = || {
        entries
            .values()
            .filter(|entry| entry.coordinator_wake.worker.launcher_board.is_some())
    };
    let working: BTreeSet<String> = workers()
        .filter(|entry| {
            matches!(
                entry.status,
                SubagentStatus::Starting | SubagentStatus::Running
            )
        })
        .filter_map(|entry| {
            let board = entry.coordinator_wake.worker.launcher_board.as_ref()?;
            Some(board.member.clone())
        })
        .collect();
    let stranded = stranded_work(work, &working);
    let signature = stranded.as_ref().map(Stranded::signature);
    let reported = workers().any(|entry| {
        entry.coordinator_wake.worker.stall_reported.is_some()
            && entry.coordinator_wake.worker.stall_reported == signature
    });
    record_stall(entries.values_mut(), signature);
    match reported {
        true => None,
        false => stranded,
    }
}

fn record_stall<'a>(
    entries: impl Iterator<Item = &'a mut SubagentEntry>,
    signature: Option<String>,
) {
    for entry in entries {
        let worker = &mut entry.coordinator_wake.worker;
        if worker.launcher_board.is_some() {
            worker.stall_reported = signature.clone();
        }
    }
}
