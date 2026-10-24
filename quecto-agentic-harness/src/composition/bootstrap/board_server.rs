//! The board server's graph (epic #2705 B4, B5, B7): the SQLite board in
//! the state directory, written one operation at a time; the clone's git,
//! for the board's id and its remote's revisions and pull requests; this
//! machine's clock; peer credentials letting in this user only; the tokens
//! file in the state directory; and the TCP addresses the global config
//! names.
use std::path::Path;
use std::sync::Arc;

use super::project_board::{operation_parts, pull_request_states};
use super::project_board_operations::LocalBoardOperations;
use crate::application::project_board::dto::{NoPullRequestHost, ProjectBoardError};
use crate::application::project_board::operations::StoreListings;
use crate::application::project_board::ports::ListenAddress;
use crate::application::project_board::ports::{
    BoardClock, BoardOpStore, BoardOperations, BoardOperationsOver, BoardTokens, ProjectBoardStore,
    PullRequestStates, RemoteRevisions,
};
use crate::application::project_board::use_cases::{
    BoardAccess, QueryBoard, RunBoardOperation, ServeBoard, ServeBoardOperation,
};
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::shared::redaction::{redact_secrets, redact_url_userinfo};
use crate::infrastructure::config::Config;
use crate::infrastructure::http::board_service::listener::{SocketPeerCredentials, this_uid};
use crate::infrastructure::persistence::file::board_tokens::FileBoardTokens;
use crate::infrastructure::persistence::project_board::board_tree::told;
use crate::infrastructure::persistence::project_board::git::{Git, GitEnvironment};
use crate::infrastructure::persistence::project_board::remote_revisions::GitRemoteRevisions;
use crate::infrastructure::persistence::project_board::store::{GitBoardStore, StoreOpenError};
use crate::infrastructure::persistence::sqlite::project_board::SqliteBoardOps;
use crate::infrastructure::time::board_clock::SystemBoardClock;
pub use crate::interface::cli::board_server::{BoardServerEnvironment, BoardServerHandles};

/// Builds the board server for `environment`'s clone and board remote.
/// # Errors
/// No git, remote, URL or board id; a bad config or address; a state
/// directory or tokens file refused (secrets and URL userinfo redacted).
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
    let ops =
        SqliteBoardOps::open(&state.join(BOARD_FILE), server_identity()).map_err(|error| {
            refused(&format!(
                "the board in {}: {}",
                state.display(),
                told(&error)
            ))
        })?;
    let ops: Arc<dyn BoardOpStore> = Arc::new(ops);
    let clock: Arc<dyn BoardClock> = Arc::new(SystemBoardClock::default());
    let served = ServedOperations {
        clock: Arc::clone(&clock),
        prs: pull_request_states(&url, &environment.vars),
        revisions: Arc::new(GitRemoteRevisions::of_store(&store)),
    };
    Ok(BoardServerHandles {
        serve: ServeBoard::new(ops.reads(), Arc::clone(&clock), board),
        operations: ServeBoardOperation::new(ops, Arc::new(served)),
        access: BoardAccess::new(tokens, clock),
        peers: Arc::new(SocketPeerCredentials),
        uid: this_uid(),
        listen,
    })
}

/// The board's file in the state directory.
pub const BOARD_FILE: &str = "board.sqlite3";

/// Who the server's reads are made as: a read writes nothing, and every
/// write is its caller's ([`OpWrite`](crate::application::project_board::ports::OpWrite)).
fn server_identity() -> Identity {
    Identity {
        name: "quecto board-server".into(),
        email: "board-server@quecto.invalid".into(),
    }
}

/// The board's operations as a server runs them (B7), over the store each
/// is handed: this machine's clock, pull request states read with `gh`
/// for the board's remote, a success judged by the remote's revisions. A
/// start launches nothing and no run's swarm is seen here (B8).
pub struct ServedOperations {
    pub clock: Arc<dyn BoardClock>,
    pub prs: Result<Arc<dyn PullRequestStates>, NoPullRequestHost>,
    pub revisions: Arc<dyn RemoteRevisions>,
}

impl BoardOperationsOver for ServedOperations {
    fn over(
        &self,
        store: Arc<dyn ProjectBoardStore>,
    ) -> Result<Box<dyn BoardOperations>, ProjectBoardError> {
        let listings = StoreListings(QueryBoard::new(Arc::clone(&store)));
        let parts = operation_parts(
            (Arc::clone(&store), store),
            Arc::clone(&self.clock),
            Arc::new(listings),
            self.prs.clone(),
            Arc::clone(&self.revisions),
            None,
        )?;
        let operations = LocalBoardOperations::new(RunBoardOperation::new(parts));
        Ok(Box::new(operations))
    }
}

/// The addresses `board_server.listen` names in the config at `path`.
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
