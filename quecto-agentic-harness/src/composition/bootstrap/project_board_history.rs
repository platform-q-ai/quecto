//! Composition-only native history export/read graphs.
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

/// Offline export graph: no listener, token store, network or writable DB open.
pub fn build_history_cli(
    e: crate::interface::cli::board_server::history::HistoryCliEnvironment,
) -> Result<crate::interface::cli::board_server::history::HistoryCliHandles, String> {
    use crate::infrastructure::persistence::project_board::git::{Git, GitEnvironment};
    use crate::interface::cli::board_server::history::HistoryCliHandles;
    let data = dirs::data_dir()
        .ok_or("no data directory")?
        .join("quecto")
        .join("boards");
    let git = Git::reader(&e.repo, GitEnvironment::of_process()).map_err(|e| e.to_string())?;
    let valid = git
        .local(&["check-ref-format", &e.target], None, 1024)
        .map_err(|e| e.to_string())?;
    if valid.status.success() {
    } else {
        return Err("invalid destination ref".into());
    }
    let captured = git
        .local(&["rev-parse", "--verify", "--quiet", &e.target], None, 1024)
        .map_err(|e| e.to_string())?;
    let expected = match (captured.status.success(), captured.status.code()) {
        (true, _) => Some(
            String::from_utf8(captured.stdout)
                .map_err(|e| e.to_string())?
                .trim()
                .to_owned(),
        ),
        (false, Some(1)) => None,
        _ => return Err("cannot capture destination lease".into()),
    };
    let handles = build_board_history(BoardHistoryEnvironment {
        destination_repo: e.repo,
        database: data.join(format!("{}.sqlite", e.board.as_str())),
        target_ref: e.target,
        board: e.board,
        writer: Identity {
            name: "Quecto".into(),
            email: "board@quecto.invalid".into(),
        },
    });
    Ok(HistoryCliHandles {
        export: handles.export,
        expected,
    })
}
