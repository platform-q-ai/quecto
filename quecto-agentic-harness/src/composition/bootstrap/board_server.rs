//! The board server's graph (epic #2705 B4): the git store, read only
//! (writes are B7), its blobs for the files, this machine's clock, and
//! peer credentials letting in this user only. A SQLite store (B2) slots
//! in here with no server change.
use std::sync::Arc;

use crate::application::project_board::ports::{BoardClock, ProjectBoardStore, SnapshotFiles};
use crate::application::project_board::use_cases::ServeBoard;
use crate::domain::shared::redaction::{redact_secrets, redact_url_userinfo};
use crate::infrastructure::http::board_service::listener::{SocketPeerCredentials, this_uid};
use crate::infrastructure::persistence::project_board::board_tree::told;
use crate::infrastructure::persistence::project_board::git::{Git, GitEnvironment};
use crate::infrastructure::persistence::project_board::snapshot_files::GitSnapshotFiles;
use crate::infrastructure::persistence::project_board::store::{GitBoardStore, StoreOpenError};
use crate::infrastructure::time::board_clock::SystemBoardClock;
pub use crate::interface::cli::board_server::{BoardServerEnvironment, BoardServerHandles};

/// Builds the board server for `environment`'s clone and board remote.
/// # Errors
/// No git, no such remote, or one with no URL or board id: each said,
/// secrets and URL userinfo redacted.
pub fn build_board_server(
    environment: &BoardServerEnvironment,
) -> Result<BoardServerHandles, String> {
    let redacted = |why: &str| redact_secrets(&redact_url_userinfo(why));
    let refused = |why: &str| format!("board-server: {}", redacted(why));
    let vars = GitEnvironment::allowlisted(environment.vars.iter().cloned());
    let git = Git::reader(&environment.repo, vars).map_err(|error| {
        refused(&format!(
            "git cannot run in {}: {error}",
            environment.repo.display()
        ))
    })?;
    // The snapshot reads the very blobs the store reads, in its repository.
    let files: Arc<dyn SnapshotFiles> = Arc::new(GitSnapshotFiles::new(git.clone()));
    let store = GitBoardStore::new(git, &environment.remote).map_err(|error| match error {
        StoreOpenError::NoSuchRemote(why) => refused(&why),
        StoreOpenError::Unavailable(error) => refused(&told(&error)),
    })?;
    let url = store.remote_url().map_err(|error| refused(&told(&error)))?;
    // The id is made from the remote's URL and the clone's top level, as
    // `quecto board` makes it, so both name one board alike.
    let top_level = store
        .top_level()
        .unwrap_or_else(|_| environment.repo.clone());
    let board = GitBoardStore::board_id_of(&url, &top_level).ok_or_else(|| {
        refused(&format!(
            "the remote '{}' gives no board id",
            environment.remote
        ))
    })?;
    let store: Arc<dyn ProjectBoardStore> = Arc::new(store);
    let clock: Arc<dyn BoardClock> = Arc::new(SystemBoardClock::default());
    Ok(BoardServerHandles {
        serve: ServeBoard::new(store, clock, files, board),
        peers: Arc::new(SocketPeerCredentials),
        uid: this_uid(),
    })
}

#[cfg(test)]
#[path = "board_server_tests.rs"]
mod tests;
