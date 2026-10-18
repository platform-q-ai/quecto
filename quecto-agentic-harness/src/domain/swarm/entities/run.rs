//! A run's snapshot and its members, as the lifecycle reads them.

use crate::domain::swarm::value_objects::lifecycle::{MemberStatus, ProcessIdentity, RunStatus};

#[derive(Clone, Debug)]
pub struct Member {
    pub id: String,
    pub status: MemberStatus,
    pub process: Option<ProcessIdentity>,
    pub endpoint: Option<String>,
    /// The member whose harness launched this one (#2121); `None` for the
    /// bootstrapped coordinator and legacy stores.
    pub launcher: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub control_generation: u64,
    pub status: RunStatus,
    /// The outcome a paused run holds after the coordinator ended it, a
    /// budget ran out, the deadline passed or its harness was lost (#1729);
    /// a closed run keeps the outcome it reached. `None` for a plain
    /// supervisor pause, a cancelled run and every live status.
    pub outcome: Option<RunStatus>,
    pub coordinator: String,
    pub deadline: f64,
    pub members: Vec<Member>,
}

impl Snapshot {
    /// A run its coordinator ended (or that ran out of budget), waiting for
    /// the supervisor outside the swarm to resume or close it.
    pub fn ended(&self) -> bool {
        self.status == RunStatus::Paused && self.outcome.is_some_and(RunStatus::proposable)
    }

    /// Whether `actor` may run model inference: an ended run keeps its
    /// coordinator available for reporting, exactly like a terminal one.
    pub fn admits_inference(&self, actor: &str) -> bool {
        let coordinator = actor == self.coordinator;
        if self.ended() {
            coordinator
        } else {
            self.status.admits_inference(coordinator)
        }
    }
}
