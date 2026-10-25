//! Composition-only native history export/read graphs.
use super::board_server::{board_database, board_state_dir};
use crate::application::project_board::ports::history::*;
use crate::application::project_board::use_cases::ExportBoardHistory;
use crate::domain::project_board::{entities::claim::Identity, value_objects::telemetry::BoardId};
use crate::infrastructure::persistence::file::state_dir::{absolute_dir, locked, private_dir};
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
    // The board file `serve` keeps (#2741): export reads, never makes, it,
    // through an absolute path (a read needs no lock).
    absolute_dir(&board_state_dir(&e.base_dir, e.state.as_deref(), &e.board))?;
    let database = board_database(&e.base_dir, e.state.as_deref(), &e.board);
    if database.is_file() {
    } else {
        return Err(format!("no board file at {}", database.display()));
    }
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
        database,
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

/// Complete additive import graph, reusing the working export graph.
pub struct BoardHistoryImportHandles {
    pub source: Arc<dyn BoardHistorySource>,
    pub store: Arc<dyn BoardHistoryStore>,
    pub sink: Arc<dyn BoardHistorySink>,
    pub import: crate::application::project_board::use_cases::ImportBoardHistory,
    pub export: ExportBoardHistory,
}
pub fn build_board_history_import_from_ports(
    source: Arc<dyn BoardHistorySource>,
    store: Arc<dyn BoardHistoryStore>,
    sink: Arc<dyn BoardHistorySink>,
) -> BoardHistoryImportHandles {
    BoardHistoryImportHandles {
        import: crate::application::project_board::use_cases::ImportBoardHistory::new(
            source.clone(),
            store.clone(),
            ExportBoardHistory::new(store.clone(), sink.clone()),
        ),
        export: ExportBoardHistory::new(store.clone(), sink.clone()),
        source,
        store,
        sink,
    }
}

/// What import is told while a server holds the state directory.
const IMPORT_HELD: &str = "a board server holds it: stop the server before importing";

/// Offline import composes the same owned store and complete dry exporter.
pub fn build_history_import_cli(
    e: crate::interface::cli::board_server::history::ImportCliEnvironment,
) -> Result<crate::application::project_board::use_cases::ImportBoardHistory, String> {
    use crate::infrastructure::persistence::project_board::history::GitBoardHistorySource;
    // The board file `serve` keeps (#2741), through serve's allowlist, its
    // directory locked as a server locks it until the import is dropped.
    let state = board_state_dir(&e.base_dir, e.state.as_deref(), &e.board);
    private_dir(&state)?;
    let lock = locked(&state, IMPORT_HELD)?;
    let store = Arc::new(
        SqliteBoardHistoryStore::new(
            board_database(&e.base_dir, e.state.as_deref(), &e.board),
            Identity {
                name: "Quecto".into(),
                email: "board@quecto.invalid".into(),
            },
            e.board,
        )
        .holding(Box::new(lock)),
    );
    // Import only dry-proves: this sink never publishes or writes source objects.
    Ok(build_board_history_import_from_ports(
        Arc::new(GitBoardHistorySource::new(e.repo.clone(), e.remote)),
        store,
        Arc::new(GitBoardHistorySink::new(
            e.repo,
            "refs/heads/quecto/board".into(),
        )),
    )
    .import)
}

#[cfg(test)]
#[path = "project_board_history_tests.rs"]
mod tests;
