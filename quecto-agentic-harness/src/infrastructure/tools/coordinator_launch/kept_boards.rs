//! The boards of swarm environments as their member left (#2481 S4c
//! follow-up), kept in this process under the leaving member's agent id:
//! the final-member teardown keeps one before it tears the box down (its
//! checkout, and the board in it, go with the box), and the board's
//! coordinator launcher reads its own coordinator's back, so `end_run`
//! records how the run's swarm ended rather than a swarm that reported
//! nothing.
//!
//! Bounded: only the newest [`MAX_KEPT`] are kept, so a long-lived process
//! never grows without end; one past it costs the oldest. Kept in memory
//! only: a process that restarts between a coordinator's stop and its
//! run's end keeps none, and that end records nothing reported.
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::Value;

use crate::application::environments::dto::SwarmBoardSnapshot;
use crate::application::environments::ports::EndedSwarmBoards;

/// The most boards one process keeps.
pub const MAX_KEPT: usize = 16;

/// Boards kept by member, oldest first.
#[derive(Debug, Default)]
pub struct KeptSwarmBoards {
    boards: Mutex<VecDeque<(String, Value)>>,
}

impl KeptSwarmBoards {
    /// The board kept for `member`, if one is.
    pub fn board(&self, member: &str) -> Option<Value> {
        let boards = self.boards.lock().unwrap_or_else(|e| e.into_inner());
        boards
            .iter()
            .find(|(kept, _)| kept == member)
            .map(|(_, board)| board.clone())
    }
}

impl EndedSwarmBoards for KeptSwarmBoards {
    fn keep(&self, member: &str, board: SwarmBoardSnapshot) {
        let mut boards = self.boards.lock().unwrap_or_else(|e| e.into_inner());
        // The newest board of a member is the one kept.
        boards.retain(|(kept, _)| kept != member);
        boards.push_back((member.to_owned(), board.into_value()));
        while boards.len() > MAX_KEPT {
            boards.pop_front();
        }
        assert!(
            (1..=MAX_KEPT).contains(&boards.len()),
            "a keep leaves between one and {MAX_KEPT} boards"
        );
    }
}

/// The boards this process keeps: the one store the teardown keeps into
/// and every launcher reads from.
pub fn process_kept_boards() -> Arc<KeptSwarmBoards> {
    static KEPT: OnceLock<Arc<KeptSwarmBoards>> = OnceLock::new();
    Arc::clone(KEPT.get_or_init(Arc::default))
}

#[cfg(test)]
#[path = "kept_boards_tests.rs"]
mod tests;
