//! The board server's graph (epic #2705 B4, B5): the git store, read only
//! (writes are B7), this machine's clock, peer credentials letting in this
//! user only, the device tokens in a file under the state directory, and
//! the TCP addresses the global config names. A SQLite store (B2) slots in
//! here with no server change, and later keeps the tokens too.
use std::path::Path;
use std::sync::Arc;

use crate::application::project_board::ports::ListenAddress;
use crate::application::project_board::ports::{BoardClock, BoardTokens, ProjectBoardStore};
use crate::application::project_board::use_cases::{BoardAccess, ServeBoard};
use crate::domain::shared::redaction::{redact_secrets, redact_url_userinfo};
use crate::infrastructure::config::Config;
use crate::infrastructure::http::board_service::listener::{SocketPeerCredentials, this_uid};
use crate::infrastructure::persistence::file::board_tokens::FileBoardTokens;
use crate::infrastructure::persistence::project_board::board_tree::told;
use crate::infrastructure::persistence::project_board::git::{Git, GitEnvironment};
use crate::infrastructure::persistence::project_board::store::{GitBoardStore, StoreOpenError};
use crate::infrastructure::time::board_clock::SystemBoardClock;
pub use crate::interface::cli::board_server::{BoardServerEnvironment, BoardServerHandles};

/// Builds the board server for `environment`'s clone and board remote.
/// # Errors
/// No git, no such remote, or one with no URL or board id; a config that
/// does not load or names an address off the allowlist; a state directory
/// or tokens file refused: each said, secrets and URL userinfo redacted.
pub fn build_board_server(
    environment: &BoardServerEnvironment,
) -> Result<BoardServerHandles, String> {
    let redacted = |why: &str| redact_secrets(&redact_url_userinfo(why));
    let refused = |why: &str| format!("board-server: {}", redacted(why));
    let listen = configured_listen(&environment.config).map_err(|why| refused(&why))?;
    let vars = GitEnvironment::allowlisted(environment.vars.iter().cloned());
    let git = Git::reader(&environment.repo, vars).map_err(|error| {
        refused(&format!(
            "git cannot run in {}: {error}",
            environment.repo.display()
        ))
    })?;
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
    let state = environment.state.clone().unwrap_or_else(|| {
        environment
            .base_dir
            .join("board-server")
            .join(board.as_str())
    });
    let tokens: Arc<dyn BoardTokens> =
        Arc::new(FileBoardTokens::open(&state).map_err(|why| refused(&why))?);
    let store: Arc<dyn ProjectBoardStore> = Arc::new(store);
    let clock: Arc<dyn BoardClock> = Arc::new(SystemBoardClock::default());
    Ok(BoardServerHandles {
        serve: ServeBoard::new(store, Arc::clone(&clock), board),
        access: BoardAccess::new(tokens, clock),
        peers: Arc::new(SocketPeerCredentials),
        uid: this_uid(),
        listen,
    })
}

/// The addresses `board_server.listen` in the global config at `path`
/// names; none when the file or the key is missing.
fn configured_listen(path: &Path) -> Result<Vec<ListenAddress>, String> {
    let shown = path.display();
    let text = path
        .to_str()
        .ok_or_else(|| format!("the config path {shown} is not UTF-8"))?;
    let config = Config::load(text).map_err(|error| format!("the config {shown}: {error}"))?;
    config
        .board_server
        .unwrap_or_default()
        .listen
        .iter()
        .map(|address| {
            ListenAddress::parse(address)
                .map_err(|why| format!("board_server.listen in {shown}: {why}"))
        })
        .collect()
}

#[cfg(test)]
#[path = "board_server_tests.rs"]
mod tests;
