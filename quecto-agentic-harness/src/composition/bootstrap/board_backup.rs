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

/// Resolves the same board file as serve/import/export, without creating state.
pub fn build_backup_cli(
    environment: crate::interface::cli::board_server::backup::BackupCliEnvironment,
) -> Result<BackupBoard, String> {
    use super::board_server::board_database;
    use crate::{
        domain::project_board::value_objects::telemetry::BoardId,
        infrastructure::{
            persistence::sqlite::project_board::backup::SqliteBoardBackup,
            time::board_clock::SystemBoardClock,
        },
    };
    if environment
        .state
        .as_ref()
        .is_none_or(|state| state.is_absolute() && environment.board.is_some())
    {
    } else {
        return Err("backup: absolute --state requires --board-id".into());
    }
    // The placeholder locates only the discovery parent through the sole resolver;
    // no source is selected until a real canonical directory ID is admitted.
    let board = BoardId::of_hex(environment.board.as_deref().unwrap_or("0000000000000000"))
        .ok_or("backup: invalid board id")?;
    let database = board_database(&environment.base_dir, environment.state.as_deref(), &board);
    let parent = database.parent().expect("resolved board file has a parent");
    let single = environment.state.as_ref().and(environment.board.clone());
    let root = if single.is_some() {
        parent
    } else {
        parent.parent().expect("default board state has a parent")
    }
    .to_owned();
    let resolve = Arc::new(move |id: &str| {
        let board = BoardId::of_hex(id).expect("adapter admits canonical board ids");
        board_database(&environment.base_dir, environment.state.as_deref(), &board)
    });
    Ok(build_backup_from_ports(
        Arc::new(SqliteBoardBackup::with_resolver(root, single, resolve)),
        Arc::new(SystemBoardClock::default()),
    ))
}
