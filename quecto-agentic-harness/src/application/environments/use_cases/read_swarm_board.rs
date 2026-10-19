//! The live board view a supervising client reads on its parent's socket
//! (#2475, #2478): each swarm this session hosts, by its environment ref,
//! read through the board's own adapter.
//!
//! Scope (#2488 review M2): this session's own environments only, as
//! `get_containers` lists them without `all` (`is_own`: created here, or
//! restored from an earlier run of the same named session), and only those
//! that may still hold a board (running or retained). The registry is
//! seeded from every session's records, so another session's swarm is
//! refused by name, never read.
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::application::environments::dto::{BoardSighting, SwarmBoardRefusal, SwarmBoardSnapshot};
use crate::application::environments::ports::hosted_swarm::HostedSwarmBoard;
use crate::domain::environments::entities::environment_registry::{
    EnvironmentLookupError, EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus,
    EnvironmentTarget, ref_number,
};
use crate::domain::environments::services::environment_listing::is_own;

pub struct ReadSwarmBoard {
    registry: EnvironmentRegistry,
    boards: Arc<dyn HostedSwarmBoard>,
}

impl ReadSwarmBoard {
    pub fn new(registry: EnvironmentRegistry, boards: Arc<dyn HostedSwarmBoard>) -> Self {
        Self { registry, boards }
    }

    /// The board of the environment `swarm` names (`C1`): its projection.
    ///
    /// # Errors
    /// Not an environment ref; an unknown ref or an unreadable registry;
    /// another session's environment; one neither running nor retained;
    /// one hosting no reachable board; or the board's own refusal.
    pub async fn board(&self, swarm: &str) -> Result<SwarmBoardSnapshot, SwarmBoardRefusal> {
        // An allowlist of the ref grammar: anything else is never looked
        // up, nor echoed.
        if ref_number(swarm).is_none() {
            return Err(SwarmBoardRefusal::NotARef);
        }
        let record = self
            .registry
            .resolve(&EnvironmentTarget::Ref(swarm.to_owned()))
            .map_err(|error| match error {
                EnvironmentLookupError::Unknown(_) => {
                    SwarmBoardRefusal::UnknownSwarm(swarm.to_owned())
                }
                unreadable @ (EnvironmentLookupError::Unreadable(_)
                | EnvironmentLookupError::Ambiguous(_)
                | EnvironmentLookupError::Stopped(_)
                | EnvironmentLookupError::Stale(_)) => {
                    SwarmBoardRefusal::Unreadable(unreadable.to_string())
                }
            })?;
        // A ref resolves to itself; any other record is refused, never a
        // panic on a client's path.
        match record.environment_ref == swarm {
            true => {}
            false => {
                return Err(SwarmBoardRefusal::Unreadable(format!(
                    "the registry answered {} for {swarm}",
                    record.environment_ref
                )));
            }
        }
        if !is_own(&record, self.registry.session()) {
            return Err(SwarmBoardRefusal::OtherSession(swarm.to_owned()));
        }
        if !holds_a_board(&record) {
            return Err(SwarmBoardRefusal::NotLive(swarm.to_owned()));
        }
        match self.boards.board(&record).await {
            Ok(Some(board)) => Ok(board),
            Ok(None) => Err(SwarmBoardRefusal::NoBoard(swarm.to_owned())),
            Err(error) => Err(SwarmBoardRefusal::Unreadable(error)),
        }
    }

    /// What a poll finds at each of this session's environments that may
    /// still hold a board (running or retained): its board's revision, or
    /// why it could not be read. One that holds no board is left out.
    pub async fn sightings(&self) -> BTreeMap<String, BoardSighting> {
        let session = self.registry.session().to_owned();
        let mut sightings = BTreeMap::new();
        let polled = self
            .registry
            .entries()
            .into_iter()
            .filter(|record| is_own(record, &session) && holds_a_board(record));
        for record in polled {
            let sighting = match self.boards.revision(&record).await {
                Ok(Some(revision)) => BoardSighting::Board(revision),
                Ok(None) => continue,
                Err(error) => BoardSighting::Unreadable(error),
            };
            sightings.insert(record.environment_ref, sighting);
        }
        sightings
    }
}

/// Running environments and retained ones (whose board outlives the run)
/// may hold a board; stopping or stopped ones do not.
fn holds_a_board(record: &EnvironmentRecord) -> bool {
    matches!(
        record.status,
        EnvironmentStatus::Running | EnvironmentStatus::Retained
    )
}

#[cfg(test)]
#[path = "read_swarm_board_tests.rs"]
mod tests;
