//! Offline maintenance construction; never builds the token/listener graph.
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

/// Uses B3's data root without creating it or constructing credentials/listeners.
pub fn build_backup_cli() -> Result<BackupBoard, String> {
    use crate::infrastructure::{
        persistence::sqlite::project_board::backup::SqliteBoardBackup,
        time::board_clock::SystemBoardClock,
    };
    let root = dirs::data_dir()
        .ok_or("backup: no data directory")?
        .join("quecto")
        .join("boards");
    Ok(build_backup_from_ports(
        Arc::new(SqliteBoardBackup::new(root)),
        Arc::new(SystemBoardClock::default()),
    ))
}
