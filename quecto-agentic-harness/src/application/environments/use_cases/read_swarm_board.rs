//! The live board view a supervising client reads on its parent's socket
//! (#2475, #2478): each swarm this session hosts, by its environment ref,
//! read through the board's own adapter.
use std::sync::Arc;

use serde_json::Value;

use crate::application::environments::dto::SwarmBoardRefusal;
use crate::application::environments::ports::HostedSwarmBoard;
use crate::domain::environments::entities::environment_registry::{
    EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus,
};

pub struct ReadSwarmBoard {
    registry: EnvironmentRegistry,
    boards: Arc<dyn HostedSwarmBoard>,
}

impl ReadSwarmBoard {
    pub fn new(registry: EnvironmentRegistry, boards: Arc<dyn HostedSwarmBoard>) -> Self {
        Self { registry, boards }
    }

    /// The board of the environment `swarm` names (`C1`).
    ///
    /// # Errors
    /// An unknown ref, an environment hosting no reachable board, or the
    /// board's own refusal.
    pub async fn board(&self, swarm: &str) -> Result<Value, SwarmBoardRefusal> {
        let record = self
            .registry
            .get(swarm)
            .ok_or_else(|| SwarmBoardRefusal::UnknownSwarm(swarm.to_owned()))?;
        match self.boards.board(&record).await {
            Ok(Some(board)) => Ok(board),
            Ok(None) => Err(SwarmBoardRefusal::NoBoard(swarm.to_owned())),
            Err(error) => Err(SwarmBoardRefusal::Unreadable(error)),
        }
    }

    /// Each hosted board's revision, by environment ref: every environment
    /// that may still hold a board (running or retained) and holds one now.
    /// One that cannot be read is left out until it can.
    pub async fn revisions(&self) -> Vec<(String, i64)> {
        let mut revisions = Vec::new();
        for record in self.registry.entries().into_iter().filter(holds_a_board) {
            if let Ok(Some(revision)) = self.boards.revision(&record).await {
                revisions.push((record.environment_ref, revision));
            }
        }
        revisions
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
