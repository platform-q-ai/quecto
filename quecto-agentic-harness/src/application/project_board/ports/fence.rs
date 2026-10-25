//! The board fence publication capability.
use super::{BoardHead, BoardStoreError};
use crate::domain::project_board::value_objects::board_moved::BoardMoved;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FenceReport {
    Published {
        previous: BoardHead,
        head: BoardHead,
    },
    Already {
        head: BoardHead,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FenceError {
    Store(BoardStoreError),
    Conflict,
    Refused(String),
    Uncertain(String),
    Unavailable(String),
}
/// Publication capability, not the run-evidence RemoteRevisions capability.
pub trait BoardFence: Send + Sync {
    fn publish_fence(
        &self,
        captured: &BoardHead,
        moved: &BoardMoved,
    ) -> Result<FenceReport, FenceError>;
}
