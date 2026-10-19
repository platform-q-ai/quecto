//! The boards of ended swarm runs as their member left its environment
//! (#2481 S4c follow-up), kept under the session and the member's spawn
//! label (`board-<task>-run-<n>` for a board run's coordinator): the
//! final-member teardown keeps one before it tears the box down (its
//! checkout, and the board in it, go with the box), and a session's
//! [`KeptRunReports`] reads its run's back by that label, so `end_run`
//! records how the run's swarm ended rather than a swarm that reported
//! nothing. Keyed by label, not by the agent a launch answered, so no
//! memory of the launch is needed; only a label shaped as a board run's
//! coordinator's is kept, so no ad-hoc swarm fills the store.
//!
//! Only what a fold-back reads is kept ([`SwarmRunView`]: the run's end,
//! its items with their states, owners and evidence, and its spend), read
//! from the board's projection as it is kept; a projection that is not a
//! whole ended run is not kept. A board is dropped once its run is
//! recorded, and only the newest [`MAX_KEPT`] are kept, so a long-lived
//! process never grows without end. Kept in memory: a process that
//! restarts between the coordinator's teardown and the run's end has lost
//! it, and a teardown that no longer knows the coordinator's label (a
//! restarted harness that does not hold it as its subagent) keeps none;
//! such an end records nothing reported.
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::application::environments::dto::SwarmBoardSnapshot;
use crate::application::environments::ports::hosted_swarm::EndedSwarmBoards;
use crate::application::project_board::dto::SwarmRunView;
use crate::application::project_board::ports::SwarmRunReports;
use crate::domain::project_board::value_objects::slug::Slug;

/// The most boards one store keeps.
pub const MAX_KEPT: usize = 16;

/// A kept board's key: the session, and the member's label.
type Key = (String, String);

/// Ended runs' boards by session and label, oldest first. Composition
/// builds one per harness and hands it to the teardown and the launcher.
#[derive(Debug, Default)]
pub struct KeptSwarmBoards {
    boards: Mutex<VecDeque<(Key, SwarmRunView)>>,
}

impl KeptSwarmBoards {
    /// The board kept for `session`'s member `label`, if one is.
    pub fn view(&self, session: &str, label: &str) -> Option<SwarmRunView> {
        let boards = self.boards.lock().unwrap_or_else(|e| e.into_inner());
        boards
            .iter()
            .find(|((kept_session, kept_label), _)| kept_session == session && kept_label == label)
            .map(|(_, view)| view.clone())
    }

    /// Drops the board kept for `session`'s member `label`, if one is.
    pub fn forget(&self, session: &str, label: &str) {
        let mut boards = self.boards.lock().unwrap_or_else(|e| e.into_inner());
        boards.retain(|((kept_session, kept_label), _)| {
            !(kept_session == session && kept_label == label)
        });
    }
}

impl EndedSwarmBoards for KeptSwarmBoards {
    fn keep(&self, session: &str, label: &str, board: SwarmBoardSnapshot) {
        // Only a board run's coordinator is kept (an allowlist of its
        // label's shape): no other swarm's board is ever read back.
        match super::is_board_label(label) {
            true => {}
            false => {
                tracing::debug!(member = %label, "not a board run's coordinator: its board is not kept");
                return;
            }
        }
        let Some(view) = super::run_report::swarm_run_view(&board.into_value()) else {
            tracing::warn!(
                member = %label,
                "an ended swarm's board was not kept: not a whole ended run"
            );
            return;
        };
        let key = (session.to_owned(), label.to_owned());
        // One guard: the old board goes and the new one comes at once.
        let mut boards = self.boards.lock().unwrap_or_else(|e| e.into_inner());
        boards.retain(|(kept, _)| *kept != key);
        boards.push_back((key, view));
        while boards.len() > MAX_KEPT {
            boards.pop_front();
        }
        assert!(
            (1..=MAX_KEPT).contains(&boards.len()),
            "a keep leaves between one and {MAX_KEPT} boards"
        );
    }
}

/// A session's runs' swarms as the teardown kept their boards: the run
/// reports port (#2481 S4c follow-up), reading each run's board by the
/// label its coordinator is spawned with, so it needs no memory of the
/// launch.
pub struct KeptRunReports {
    kept: Arc<KeptSwarmBoards>,
    session: String,
}

impl KeptRunReports {
    /// The reports `kept` holds for `session`.
    pub fn new(kept: Arc<KeptSwarmBoards>, session: &str) -> Self {
        Self {
            kept,
            session: session.to_owned(),
        }
    }
}

impl SwarmRunReports for KeptRunReports {
    fn ended_run(&self, task: &Slug, run: u32) -> Option<SwarmRunView> {
        self.kept
            .view(&self.session, &super::label(task.as_str(), run))
    }

    fn recorded(&self, task: &Slug, run: u32) {
        self.kept
            .forget(&self.session, &super::label(task.as_str(), run));
    }
}

#[cfg(test)]
#[path = "kept_boards_tests.rs"]
mod tests;
