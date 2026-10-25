//! Offline backup orchestration boundary: wall clock is injected independently
//! of source/destination resource ownership.
use crate::application::project_board::ports::{
    BoardClock,
    backup::{BackupError, BackupReport, BackupRequest, BoardBackup},
};
use std::sync::Arc;

#[allow(dead_code)]
pub struct BackupBoard {
    backup: Arc<dyn BoardBackup>,
    clock: Arc<dyn BoardClock>,
}
impl BackupBoard {
    pub fn new(backup: Arc<dyn BoardBackup>, clock: Arc<dyn BoardClock>) -> Self {
        Self { backup, clock }
    }
    pub fn execute(&self, request: &BackupRequest) -> Result<BackupReport, BackupError> {
        let _ = request;
        Err(BackupError::Unavailable("B13-lite not implemented".into()))
    }
}
