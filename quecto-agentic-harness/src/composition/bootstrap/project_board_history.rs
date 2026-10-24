//! RED-only export/read graph. No source, capture, import or acceptance behavior.
use crate::application::project_board::ports::history::*;
use crate::application::project_board::use_cases::ExportBoardHistory;
use crate::domain::project_board::{entities::claim::Identity, value_objects::telemetry::BoardId};
use crate::infrastructure::persistence::{
    project_board::history::GitBoardHistorySink,
    sqlite::project_board::history::SqliteBoardHistoryStore,
};
use std::{path::PathBuf, sync::Arc};
pub struct BoardHistoryEnvironment {
    pub destination_repo: PathBuf,
    pub database: PathBuf,
    pub target_ref: String,
    pub board: BoardId,
    pub writer: Identity,
}
pub struct BoardHistoryHandles {
    pub store: Arc<dyn BoardHistoryRead>,
    pub sink: Arc<dyn BoardHistorySink>,
    pub export: ExportBoardHistory,
}
pub fn build_board_history(environment: BoardHistoryEnvironment) -> BoardHistoryHandles {
    let store = Arc::new(SqliteBoardHistoryStore::new(
        environment.database,
        environment.writer,
        environment.board,
    ));
    let sink = Arc::new(GitBoardHistorySink::new(
        environment.destination_repo,
        environment.target_ref,
    ));
    build_board_history_from_ports(store, sink)
}
pub fn build_board_history_from_ports(
    store: Arc<dyn BoardHistoryRead>,
    sink: Arc<dyn BoardHistorySink>,
) -> BoardHistoryHandles {
    BoardHistoryHandles {
        export: ExportBoardHistory::new(store.clone(), sink.clone()),
        store,
        sink,
    }
}
