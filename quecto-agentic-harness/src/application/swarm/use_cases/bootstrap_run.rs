//! The first half of `Workbench._bootstrap` (#2270): every container's
//! placeholder run. (The second half, joining it, is membership admission.)
use std::sync::Arc;

use serde_json::Value;

use crate::application::swarm::board_operation::{atomic, detail, text};
use crate::application::swarm::dto::{
    BootstrapRunRequest, Bootstrapped, NewMember, NewRun, RunContract,
};
use crate::application::swarm::ports::{BoardRepository, Clock, IdSource};
use crate::domain::swarm::{BoardError, MemberState, RunState};

/// The placeholder's member limit.
const PLACEHOLDER_MEMBER_LIMIT: i64 = 10;

/// Creates the board when it is missing and, when it holds no run, writes
/// the placeholder: status `setup`, deadline 0 (so no swarm exists yet,
/// `domain::swarm::participates`), the bootstrapping member as coordinator
/// and integrator with a live row, and the event `container_setup`. A board
/// that already holds a run is left as it is.
pub struct BootstrapRun {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
    ids: Arc<dyn IdSource>,
}

impl BootstrapRun {
    pub fn new(
        repository: Arc<dyn BoardRepository>,
        clock: Arc<dyn Clock + Send + Sync>,
        ids: Arc<dyn IdSource>,
    ) -> Self {
        Self {
            repository,
            clock,
            ids,
        }
    }

    /// # Errors
    /// The store's refusal.
    pub fn execute(&self, request: BootstrapRunRequest) -> Result<Bootstrapped, BoardError> {
        let member = request.member.as_str();
        atomic(&*self.repository, true, |transaction| {
            if transaction.run()?.is_some() {
                return Ok(Bootstrapped { created: false });
            }
            transaction.insert_run(&NewRun {
                id: self.ids.hex32(),
                contract: RunContract {
                    goal: String::new(),
                    constraints: Value::Array(Vec::new()),
                    criteria: Value::Array(Vec::new()),
                    member_limit: PLACEHOLDER_MEMBER_LIMIT,
                    deadline: 0.0,
                },
                coordinator: member.to_owned(),
                integrator: member.to_owned(),
                status: RunState::SETUP,
            })?;
            transaction.insert_member(&NewMember {
                id: member.to_owned(),
                reservation: self.ids.hex32(),
                status: MemberState::LIVE,
                pid: request.pid,
                started: request.started.clone(),
                socket: request.socket.clone(),
                launcher: None,
            })?;
            transaction.event(
                member,
                self.clock.now_seconds(),
                "container_setup",
                &detail([("member", text(member))]),
            )?;
            Ok(Bootstrapped { created: true })
        })
    }
}

#[cfg(test)]
#[path = "bootstrap_run_tests.rs"]
mod tests;
