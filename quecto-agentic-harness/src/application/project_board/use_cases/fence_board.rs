//! Offline fence publication; infrastructure owns capture validation and CAS.
use crate::application::project_board::ports::{
    BoardHead,
    fence::{BoardFence, FenceError, FenceReport},
};
use crate::domain::project_board::value_objects::board_moved::BoardMoved;
use std::sync::Arc;

pub struct FenceBoard {
    publication: Arc<dyn BoardFence>,
}
impl FenceBoard {
    pub fn new(publication: Arc<dyn BoardFence>) -> Self {
        Self { publication }
    }
    pub fn execute(
        &self,
        captured: &BoardHead,
        destination: &BoardMoved,
    ) -> Result<FenceReport, FenceError> {
        self.publication.publish_fence(captured, destination)
    }
}
#[cfg(test)]
#[path = "fence_board_tests.rs"]
mod tests;
