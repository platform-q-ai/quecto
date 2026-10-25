//! The board server's graph (epic #2705 B4, B5, B7): the SQLite board and
//! tokens in the state directory, and the clone's git for the board's id.
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::project_board::{operation_parts, pull_request_states};
use super::project_board_operations::LocalBoardOperations;
use crate::application::project_board::dto::{BoardOp, BoardRequest, NoPullRequestHost};
use crate::application::project_board::operations::StoreListings;
use crate::application::project_board::ports::ListenAddress;
use crate::application::project_board::ports::{
    BoardClock, BoardOpStore, BoardOperations, BoardOperationsOver, BoardTokens, OperationsOver,
    ProjectBoardStore, PullRequestState, PullRequestStates, PullRequestStatesError,
    RemoteRevisions, TimeSync,
};
use crate::application::project_board::synced_clock::SyncedBoardClock;
use crate::application::project_board::use_cases::{
    BoardAccess, QueryBoard, READ_BUDGET, RunBoardOperation, ServeBoard, ServeBoardOperation,
};
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;
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
use crate::infrastructure::time::kernel_time_sync::KernelTimeSync;
pub use crate::interface::cli::board_server::{BoardServerEnvironment, BoardServerHandles};

/// Builds the board server for `environment`'s clone and board remote,
/// its clock gated on the kernel's word ([`build_board_server_with`]).
/// # Errors
/// As [`build_board_server_with`].
pub fn build_board_server(
    environment: &BoardServerEnvironment,
) -> Result<BoardServerHandles, String> {
    build_board_server_with(environment, Arc::new(KernelTimeSync))
}

/// Builds the board server for `environment`'s clone and board remote. Its
/// operations and its time read a clock that tells the time only while
/// `sync` says the host's clock is synchronised; tokens are dated by the
/// host's clock as it is.
/// # Errors
/// No git, remote, URL or board id; a bad config or address; a state
/// directory or tokens file refused (secrets and URL userinfo redacted).
pub fn build_board_server_with(
    environment: &BoardServerEnvironment,
    sync: Arc<dyn TimeSync>,
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
    let host: Arc<dyn BoardClock> = Arc::new(SystemBoardClock::default());
    let clock: Arc<dyn BoardClock> = Arc::new(SyncedBoardClock::new(Arc::clone(&host), sync));
    let served = ServedOperations {
        clock: Arc::clone(&clock),
        prs: pull_request_states(&url, &environment.vars),
        revisions: Arc::new(GitRemoteRevisions::of_store(&store)),
        read_budget: READ_BUDGET,
    };
    Ok(BoardServerHandles {
        serve: ServeBoard::new(ops.reads(), Arc::clone(&clock), board),
        operations: ServeBoardOperation::new(ops, Arc::new(served)),
        access: BoardAccess::new(tokens, host),
        peers: Arc::new(SocketPeerCredentials),
        uid: this_uid(),
        listen,
    })
}

/// The board's file in the state directory.
pub const BOARD_FILE: &str = "board.sqlite3";

/// Who the server's reads are made as (every write is its caller's).
fn server_identity() -> Identity {
    Identity {
        name: "quecto board-server".into(),
        email: "board-server@quecto.invalid".into(),
    }
}

/// The board's operations as a server runs them (B7): no launcher (B8).
pub struct ServedOperations {
    pub clock: Arc<dyn BoardClock>,
    pub prs: Result<Arc<dyn PullRequestStates>, NoPullRequestHost>,
    pub revisions: Arc<dyn RemoteRevisions>,
    pub read_budget: Duration,
}

impl BoardOperationsOver for ServedOperations {
    fn prepare(
        &self,
        request: Option<&BoardRequest>,
        reads: &dyn ProjectBoardStore,
    ) -> OperationsOver {
        let prs = self.prs.clone().map(|host| {
            let stated = StatedPullRequests::read(&*host, request, reads, self.read_budget);
            Arc::new(stated) as Arc<dyn PullRequestStates>
        });
        let (clock, revisions) = (Arc::clone(&self.clock), Arc::clone(&self.revisions));
        Box::new(move |store| {
            let listings = StoreListings(QueryBoard::new(Arc::clone(&store)));
            let parts = operation_parts(
                (Arc::clone(&store), store),
                Arc::clone(&clock),
                Arc::new(listings),
                prs.clone(),
                Arc::clone(&revisions),
                None,
            )?;
            let operations = LocalBoardOperations::new(RunBoardOperation::new(parts));
            Ok(Box::new(operations) as Box<dyn BoardOperations>)
        })
    }
}

/// The states of the pull requests a write may complete its task by.
struct StatedPullRequests(BTreeMap<u64, Result<PullRequestState, PullRequestStatesError>>);

impl StatedPullRequests {
    /// None for a task absent or linking none; unread past `budget`, unavailable.
    /// Read for the writes that may judge them: a move to review, a
    /// completion and done without a run (#2705 T4).
    fn read(
        host: &dyn PullRequestStates,
        request: Option<&BoardRequest>,
        reads: &dyn ProjectBoardStore,
        budget: Duration,
    ) -> Self {
        let (task, linking) = match request {
            Some(BoardRequest::Change(id, BoardOp::AddPr(pr))) => (Some(id), Some(*pr)),
            Some(
                BoardRequest::Change(id, BoardOp::Move(TaskStatus::Review, _))
                | BoardRequest::Complete(id)
                | BoardRequest::DoneWithoutRun { task: id, .. },
            ) => (Some(id), None),
            Some(_) | None => (None, None),
        };
        let board = task.and_then(|_| reads.read().ok());
        let mut tasks = board.iter().flat_map(|board| &board.tasks);
        let held = tasks.find(|held| Some(&held.fields().id) == task);
        let prs = held.into_iter().flat_map(|held| held.fields().prs.clone());
        let prs: BTreeSet<u64> = prs.chain(linking.filter(|_| held.is_some())).collect();
        let began = Instant::now();
        let late = || PullRequestStatesError::Unavailable("not read within its time budget".into());
        let state = |pr| match began.elapsed() < budget {
            true => host.state(pr),
            false => Err(late()),
        };
        Self(prs.into_iter().map(|pr| (pr, state(pr))).collect())
    }
}

impl PullRequestStates for StatedPullRequests {
    fn state(&self, pr: u64) -> Result<PullRequestState, PullRequestStatesError> {
        let unread = || format!("pull request {pr} was linked after its states were read");
        let state = self.0.get(&pr).cloned();
        state.unwrap_or_else(|| Err(PullRequestStatesError::Unavailable(unread())))
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
