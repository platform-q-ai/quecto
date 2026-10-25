//! The board server's graph (epic #2705 B4, B5, B7): the SQLite board and
//! tokens in the state directory, and the clone's git for the board's id;
//! the one resolver of the board's file for every command (#2741).
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::project_board::{RunSight, StartSide, operation_parts, pull_request_states};
use super::project_board_operations::LocalBoardOperations;
use crate::application::project_board::dto::{
    BoardOp, BoardRequest, NoPullRequestHost, RunEndOutcome,
};
use crate::application::project_board::operations::StoreListings;
use crate::application::project_board::ports::ListenAddress;
use crate::application::project_board::ports::{
    BoardClock, BoardOpStore, BoardOperations, BoardOperationsOver, BoardTokens, OperationsOver,
    ProjectBoardStore, PullRequestState, PullRequestStates, PullRequestStatesError,
    RemoteRevisions, ServedRun, TimeSync,
};
use crate::application::project_board::run_facts::{RunFacts, StatedRemoteRevisions};
use crate::application::project_board::synced_clock::{HeldSync, SyncedBoardClock};
use crate::application::project_board::use_cases::{
    BoardAccess, QueryBoard, READ_BUDGET, RunBoardOperation, ServeBoard, ServeBoardOperation,
};
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::value_objects::telemetry::BoardId;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;
use crate::domain::shared::redaction::{redact_secrets, redact_url_userinfo};
use crate::infrastructure::config::Config;
use crate::infrastructure::http::board_service::listener::{SocketPeerCredentials, this_uid};
use crate::infrastructure::persistence::file::board_tokens::FileBoardTokens;
use crate::infrastructure::persistence::project_board::board_tree::told;
use crate::infrastructure::persistence::project_board::git::{Git, GitEnvironment};
use crate::infrastructure::persistence::project_board::remote_revisions::GitRemoteRevisions;
use crate::infrastructure::persistence::project_board::store::{GitBoardStore, StoreOpenError};
use crate::infrastructure::persistence::sqlite::project_board::{BoardBinding, SqliteBoardOps};
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
    let state = board_state_dir(&environment.base_dir, environment.state.as_deref(), &board);
    let tokens: Arc<dyn BoardTokens> =
        Arc::new(FileBoardTokens::open(&state).map_err(|why| refused(&why))?);
    let database = board_database(&environment.base_dir, environment.state.as_deref(), &board);
    let ops = SqliteBoardOps::open(&database, server_identity()).map_err(|error| {
        refused(&format!(
            "the board in {}: {}",
            state.display(),
            told(&error)
        ))
    })?;
    // A `--state` directory names no board: the file is bound to this
    // server's board under the directory's lock, and a file bound to
    // another board is refused, never served as this one (#2741).
    match ops.bind(board.as_str()) {
        Ok(BoardBinding::Bound) => {}
        Ok(BoardBinding::Other(bound)) => {
            return Err(refused(&format!(
                "the board file {} holds board {bound}, not this clone's board {}: \
                 give this server its own --state <dir>",
                database.display(),
                board.as_str()
            )));
        }
        Err(error) => {
            return Err(refused(&format!(
                "the board in {}: {}",
                state.display(),
                told(&error)
            )));
        }
    }
    let ops: Arc<dyn BoardOpStore> = Arc::new(ops);
    let host: Arc<dyn BoardClock> = Arc::new(SystemBoardClock::default());
    let time = SyncedBoardClock::new(Arc::clone(&host), Arc::clone(&sync));
    let served = ServedOperations {
        clock: Arc::clone(&host),
        sync,
        prs: pull_request_states(&url, &environment.vars),
        revisions: Arc::new(GitRemoteRevisions::of_store(&store)),
        read_budget: READ_BUDGET,
    };
    Ok(BoardServerHandles {
        serve: ServeBoard::new(ops.reads(), Arc::new(time), board),
        operations: ServeBoardOperation::new(ops, Arc::new(served)),
        access: BoardAccess::new(tokens, host),
        peers: Arc::new(SocketPeerCredentials),
        uid: this_uid(),
        listen,
    })
}

/// The board's file in the state directory.
pub const BOARD_FILE: &str = "board.sqlite3";

/// The board server's state directory for `board` (#2741): `state` when
/// given (`--state`), else `<base_dir>/board-server/<board id>`. Serve,
/// import, export and backup all name the board's directory by this alone.
pub fn board_state_dir(base_dir: &Path, state: Option<&Path>, board: &BoardId) -> PathBuf {
    match state {
        Some(state) => state.to_path_buf(),
        None => {
            let id = Path::new(board.as_str());
            // A board id is 16 hex digits: one plain name, never a way out.
            let mut parts = id.components();
            assert!(
                matches!(
                    (parts.next(), parts.next()),
                    (Some(std::path::Component::Normal(_)), None)
                ),
                "a board id is one plain path name: {}",
                board.as_str()
            );
            base_dir.join("board-server").join(id)
        }
    }
}

/// The board's one SQLite file (#2741): [`BOARD_FILE`] in
/// [`board_state_dir`].
pub fn board_database(base_dir: &Path, state: Option<&Path>, board: &BoardId) -> PathBuf {
    board_state_dir(base_dir, state, board).join(BOARD_FILE)
}

/// Who the server's reads are made as (every write is its caller's).
fn server_identity() -> Identity {
    Identity {
        name: "quecto board-server".into(),
        email: "board-server@quecto.invalid".into(),
    }
}

/// The board's operations as a server runs them (B7): a start is begun
/// here and launched by its client, an end judged on its client's facts
/// and on `revisions` read before the lock (B8). Each op's clock is
/// `clock`, its time gated on `sync` as read once when the op begins.
pub struct ServedOperations {
    pub clock: Arc<dyn BoardClock>,
    pub sync: Arc<dyn TimeSync>,
    pub prs: Result<Arc<dyn PullRequestStates>, NoPullRequestHost>,
    pub revisions: Arc<dyn RemoteRevisions>,
    pub read_budget: Duration,
}

impl BoardOperationsOver for ServedOperations {
    fn prepare(
        &self,
        request: Option<&BoardRequest>,
        sent: &ServedRun,
        reads: &dyn ProjectBoardStore,
    ) -> OperationsOver {
        let prs = self.prs.clone().map(|host| {
            let stated = StatedPullRequests::read(&*host, request, reads, self.read_budget);
            Arc::new(stated) as Arc<dyn PullRequestStates>
        });
        // An end's facts are its client's, of the run it names; the
        // remote's revisions are read here, before the lock, within the
        // budget (B8). Facts with any other request are none.
        let (facts, ended) = match (request, sent) {
            (Some(BoardRequest::EndRun(end)), ServedRun::Facts(facts)) => (Some(facts), Some(end)),
            (_, ServedRun::Plain | ServedRun::Facts(_) | ServedRun::Undo(_)) => (None, None),
        };
        let succeeds = ended.is_some_and(|end| end.outcome == RunEndOutcome::Succeeded);
        let named = facts.filter(|_| succeeds).map(RunFacts::named_commits);
        let named = named.unwrap_or_default();
        let revisions = Arc::new(revisions_read(&self.revisions, named, self.read_budget));
        let run = ended.map(|end| (&end.task, end.run));
        let liveness = facts.zip(run).and_then(|(facts, run)| facts.liveness(run));
        let reports = facts.zip(run).and_then(|(facts, run)| facts.reports(run));
        let undo = match sent {
            ServedRun::Undo(undo) => Some(undo.clone()),
            ServedRun::Plain | ServedRun::Facts(_) => None,
        };
        let host = Arc::clone(&self.clock);
        let sync = Arc::clone(&self.sync);
        Box::new(move |store| {
            // The host's word once for the whole op, however often it reads the time.
            let held = Arc::new(HeldSync::read(&*sync));
            let clock: Arc<dyn BoardClock> =
                Arc::new(SyncedBoardClock::new(Arc::clone(&host), held));
            let listings = StoreListings(QueryBoard::new(Arc::clone(&store)));
            let sight = RunSight {
                revisions: Arc::clone(&revisions) as Arc<dyn RemoteRevisions>,
                liveness: liveness.clone(),
                reports: reports.clone(),
                start: StartSide::Serve(undo.clone()),
            };
            let parts = operation_parts(
                (Arc::clone(&store), store),
                clock,
                Arc::new(listings),
                prs.clone(),
                sight,
            )?;
            let operations = LocalBoardOperations::new(RunBoardOperation::new(parts));
            Ok(Box::new(operations) as Box<dyn BoardOperations>)
        })
    }
}

/// Which of `named` the remote holds, read on a thread of its own within
/// `budget`: past it, none is (the end is refused, as for a remote that
/// cannot be read), and the read is left to finish on its own, each of its
/// git calls bounded. Nothing named, nothing is read.
fn revisions_read(
    remote: &Arc<dyn RemoteRevisions>,
    named: Vec<String>,
    budget: Duration,
) -> StatedRemoteRevisions {
    if named.is_empty() {
        return StatedRemoteRevisions::unread("no revision was read before this write");
    }
    let (remote, (sent, read)) = (Arc::clone(remote), std::sync::mpsc::channel());
    std::thread::spawn(move || {
        let _ = sent.send(StatedRemoteRevisions::read(&*remote, &named));
    });
    read.recv_timeout(budget).unwrap_or_else(|_| {
        StatedRemoteRevisions::unread("the remote was not read within its time budget")
    })
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
