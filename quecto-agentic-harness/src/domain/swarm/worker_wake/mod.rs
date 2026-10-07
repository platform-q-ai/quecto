//! Whether a swarm coordinator hears its workers' turn ends (#2471). Its
//! workers report through the board (submissions, blocks, messages and
//! deaths wake it there), so a plain turn end reaches it only as a reply it
//! is owed, as the first good turn after a worker failed, or as stranded
//! work: a task claimed by a worker that is not working, or ready work with
//! no worker working at all.

use std::collections::BTreeSet;

use super::RunStatus;

/// The run as its coordinator reads it when a worker ends a turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkerBoard {
    pub status: RunStatus,
    /// Ready tasks (dependencies met).
    pub ready: i64,
    /// The board member owning each claimed task, one entry per task.
    pub claimed_by: Vec<String>,
}

/// Work no worker is doing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stranded {
    /// Owners of claimed tasks that are not working, sorted.
    pub claimed_by: Vec<String>,
    /// Ready tasks while no worker is working (0 otherwise).
    pub ready: i64,
}

impl Stranded {
    /// The same stranded work reads the same, so it is reported once.
    pub fn signature(&self) -> String {
        format!("{}|{}", self.claimed_by.join(","), self.ready)
    }
}

/// The work `board` strands when the members in `working` are the workers
/// still starting or in a turn: `None` for a run that is not live, or when
/// every claim's owner is working and ready work has a worker to take it.
pub fn stranded_work(board: &WorkerBoard, working: &BTreeSet<String>) -> Option<Stranded> {
    let live = matches!(board.status, RunStatus::Setup | RunStatus::Running);
    let mut claimed_by: Vec<String> = board
        .claimed_by
        .iter()
        .filter(|owner| !working.contains(owner.as_str()))
        .cloned()
        .collect();
    claimed_by.sort();
    let ready = match working.is_empty() {
        true => board.ready.max(0),
        false => 0,
    };
    match (live, claimed_by.is_empty() && ready == 0) {
        (true, false) => Some(Stranded { claimed_by, ready }),
        (true, true) | (false, _) => None,
    }
}

/// What a worker's plain turn end becomes at its coordinator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerTurnEnd {
    /// The ordinary turn-end note.
    Ordinary,
    /// Read the board: a note only for stranded work.
    CheckBoard,
}

/// `reply_due`: the coordinator sent this turn its instruction.
/// `after_failure`: the worker's previous turn failed, so its parent's
/// note queue still remembers that failure.
pub fn settle_worker_turn_end(reply_due: bool, after_failure: bool) -> WorkerTurnEnd {
    match (reply_due, after_failure) {
        (false, false) => WorkerTurnEnd::CheckBoard,
        (true, _) | (false, true) => WorkerTurnEnd::Ordinary,
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
