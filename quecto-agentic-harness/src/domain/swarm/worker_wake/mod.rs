//! Whether a swarm coordinator hears its workers' turn ends (#2471). Its
//! workers report through the board (submissions, blocks, messages and
//! deaths wake it there), so a plain turn end reaches it only as stranded
//! work: a task claimed by a worker that is not working, or ready work with
//! no worker working at all. Replies to its own instructions and failures
//! reach it by their own notes.

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
    /// The coordinator reading it: its own claims are not stranded work.
    pub coordinator: String,
}

/// Work no worker is doing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stranded {
    /// Owners of claimed tasks that are not working, sorted, one per task.
    pub claimed_by: Vec<String>,
    /// Ready tasks while no worker is working (0 otherwise).
    pub ready: i64,
}

/// The work `board` strands when the members in `working` are the workers
/// still starting or in a turn: `None` for a run that is not live, or when
/// every claim's owner is working and ready work has a worker to take it.
pub fn stranded_work(board: &WorkerBoard, working: &BTreeSet<String>) -> Option<Stranded> {
    let live = matches!(board.status, RunStatus::Setup | RunStatus::Running);
    let idle_owner = |owner: &&String| {
        working.get(owner.as_str()).is_none() && owner.as_str() != board.coordinator
    };
    let mut claimed_by: Vec<String> = board
        .claimed_by
        .iter()
        .filter(idle_owner)
        .cloned()
        .collect();
    claimed_by.sort();
    let ready = match working.is_empty() {
        true => board.ready.max(0),
        false => 0,
    };
    let stranded = Stranded { claimed_by, ready };
    match (live, stranded.claimed_by.is_empty() && stranded.ready == 0) {
        (true, false) => Some(stranded),
        (true, true) | (false, _) => None,
    }
}

impl Stranded {
    /// Whether `now` is news after this was reported: a claim this did not
    /// name, or ready work this did not count.
    pub fn covers(&self, now: &Stranded) -> bool {
        let mut reported = self.claimed_by.clone();
        let claims_reported = now.claimed_by.iter().all(|owner| {
            let found = reported.iter().position(|named| named == owner);
            found.map(|at| reported.remove(at)).is_some()
        });
        claims_reported && now.ready <= self.ready
    }

    /// Whether `member` starting a turn re-arms the report: it is one of the
    /// workers this names, or this counted ready work while none worked.
    pub fn rearmed_by(&self, member: &str) -> bool {
        self.ready > 0 || self.claimed_by.iter().any(|owner| owner == member)
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
