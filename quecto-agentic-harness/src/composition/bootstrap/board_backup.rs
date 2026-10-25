//! Constructor-only backup wiring for permanent contracts.
use crate::application::project_board::{
    ports::{BoardClock, backup::BoardBackup},
    use_cases::BackupBoard,
};
use std::sync::Arc;
pub fn build_backup_from_ports(
    backup: Arc<dyn BoardBackup>,
    clock: Arc<dyn BoardClock>,
) -> BackupBoard {
    BackupBoard::new(backup, clock)
}
