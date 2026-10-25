//! Offline backup orchestration boundary: wall clock is injected independently
//! of source/destination resource ownership.
use crate::application::project_board::ports::{
    BoardClock,
    backup::{BackupError, BackupReport, BackupRequest, BoardBackup},
};
use std::sync::Arc;

pub struct BackupBoard {
    backup: Arc<dyn BoardBackup>,
    clock: Arc<dyn BoardClock>,
}
impl BackupBoard {
    pub fn new(backup: Arc<dyn BoardBackup>, clock: Arc<dyn BoardClock>) -> Self {
        Self { backup, clock }
    }
    pub fn execute(&self, request: &BackupRequest) -> Result<BackupReport, BackupError> {
        let mut request = request.clone();
        request.at = self.clock.now().map_err(|_| BackupError::ClockOutOfRange)?;
        self.backup.backup(&request)
    }
}
