//! The board server's graph (epic #2705 B4, B5, B7): the SQLite board and
//! tokens in the state directory, and the clone's git for the board's id;
//! the one resolver of the board's file for every command (#2741).
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::project_board::{RunSight, StartSide, operation_parts, pull_request_states};
use super::project_board_operations::LocalBoardOperations;
use crate::application::project_board::dto::{
    BoardOp, BoardRequest, NoPullRequestHost, RunEnd, RunEndOutcome,
};
use crate::application::project_board::operations::StoreListings;
use crate::application::project_board::ports::ListenAddress;
use crate::application::project_board::ports::feed::BoardFeed;
use crate::application::project_board::ports::replication::{
    AdminRows, FollowTokens, ReplicaStore, ReplicationLog, ReplicationSource,
};
use crate::application::project_board::ports::{
    BoardClock, BoardOpStore, BoardOperations, BoardOperationsOver, BoardTokens, OperationsOver,
    ProjectBoardStore, PullRequestState, PullRequestStates, PullRequestStatesError,
    RemoteRevisions, ServeError, ServedRun, TimeSync,
};
use crate::application::project_board::run_facts::StatedRemoteRevisions;
use crate::application::project_board::synced_clock::{HeldSync, SyncedBoardClock};
use crate::application::project_board::use_cases::{
    BoardAccess, FollowBoard, QueryBoard, READ_BUDGET, ReadyFollowerTokens, ReplicaTiming,
    ReplicateBoard, RunBoardOperation, ServeBoard, ServeBoardOperation, ServeReplication,
};
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::plan::Plan;
use crate::domain::project_board::entities::run::Run;
use crate::domain::project_board::value_objects::telemetry::BoardId;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;
use crate::domain::shared::redaction::{redact_secrets, redact_url_userinfo};
use crate::infrastructure::config::Config;
use crate::infrastructure::http::board_remote::endpoint_of;
use crate::infrastructure::http::board_replica::RemoteReplicationSource;
use crate::infrastructure::http::board_service::listener::{SocketPeerCredentials, this_uid};
use crate::infrastructure::http::board_watch::ThreadPauses;
use crate::infrastructure::persistence::file::board_tokens::FileBoardTokens;
use crate::infrastructure::persistence::file::token_file::read_token_file;
use crate::infrastructure::persistence::project_board::board_tree::told;
use crate::infrastructure::persistence::project_board::git::{Git, GitEnvironment};
use crate::infrastructure::persistence::project_board::remote_revisions::GitRemoteRevisions;
use crate::infrastructure::persistence::project_board::store::{GitBoardStore, StoreOpenError};
use crate::infrastructure::persistence::sqlite::project_board::{BoardBinding, SqliteBoardOps};
use crate::infrastructure::time::board_clock::SystemBoardClock;
use crate::infrastructure::time::kernel_time_sync::KernelTimeSync;
pub use crate::interface::cli::board_server::{
    BoardServerEnvironment, BoardServerHandles, FollowPrimary,
};

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
    let tokens = Arc::new(FileBoardTokens::open(&state).map_err(|why| refused(&why))?);
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
    let ops = Arc::new(ops);
    // The feed is the store's own: a write publishes as it commits (B10).
    let feed = FollowBoard::new(Arc::clone(&ops) as Arc<dyn BoardFeed>);
    let replication = ServeReplication::new(
        Arc::clone(&ops) as Arc<dyn ReplicationLog>,
        Arc::clone(&tokens) as Arc<dyn AdminRows>,
    );
    let follower = match &environment.follow {
        Some(follow) => {
            let source = replication_source(follow, &board).map_err(|why| refused(&why))?;
            // A follower holds its primary's tokens: a server with its own is
            // refused, once the rest of the start is judged sound.
            ReadyFollowerTokens::new(Arc::clone(&tokens) as Arc<dyn FollowTokens>)
                .ready()
                .map_err(|why| refused(&why))?;
            let primary = source.primary();
            let replicate = ReplicateBoard::new(
                Arc::new(source),
                Arc::clone(&ops) as Arc<dyn ReplicaStore>,
                Arc::clone(&tokens) as Arc<dyn AdminRows>,
                Arc::new(ThreadPauses),
                ReplicaTiming::PRODUCTION,
            );
            Some((Arc::new(replicate), primary))
        }
        None => None,
    };
    // A follower's operations write nothing: only its primary's batches do.
    let ops: Arc<dyn BoardOpStore> = match &follower {
        Some(_) => Arc::new(ops.standby()),
        None => ops,
    };
    let host: Arc<dyn BoardClock> = Arc::new(SystemBoardClock::default());
    let time = SyncedBoardClock::new(Arc::clone(&host), Arc::clone(&sync));
    let served = ServedOperations {
        clock: Arc::clone(&host),
        sync,
        prs: pull_request_states(&url, &environment.vars),
        revisions: Arc::new(GitRemoteRevisions::of_store(&store)),
        read_budget: READ_BUDGET,
        revision_reads: Arc::default(),
    };
    let serve = ServeBoard::new(ops.reads(), Arc::new(time), board);
    let serve = match &follower {
        Some((replicate, _)) => serve.following(replicate.status_reader()),
        None => serve,
    };
    Ok(BoardServerHandles {
        serve,
        operations: ServeBoardOperation::new(ops, Arc::new(served)),
        feed: Some(feed),
        replication: Some(replication),
        follower,
        access: BoardAccess::new(tokens as Arc<dyn BoardTokens>, host),
        peers: Arc::new(SocketPeerCredentials),
        uid: this_uid(),
        listen,
    })
}

/// A follower's primary (B11): `follow`'s URL, reached with the token in
/// its token file. The file must be this user's alone (no group or other
/// bits) and hold one token on one line; no error quotes it.
fn replication_source(
    follow: &FollowPrimary,
    board: &BoardId,
) -> Result<RemoteReplicationSource, String> {
    let token = match &follow.token_file {
        None => None,
        Some(path) => Some(read_token_file(path)?),
    };
    let client = endpoint_of(&follow.url, token.as_deref())
        .map_err(|why| format!("--follow {}: {why}", follow.url))?;
    Ok(RemoteReplicationSource::new(client, board.clone()))
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
    /// Set while a read of `revisions` runs: one at a time (B8).
    pub revision_reads: Arc<AtomicBool>,
}

/// The plan run `end.run` of `end.task` was begun on, as `reads` holds it.
fn plan_of(reads: &dyn ProjectBoardStore, end: &RunEnd) -> Option<Plan> {
    let board = reads.read().ok()?;
    let ran = |run: &&Run| (&run.fields().task, run.fields().number) == (&end.task, end.run);
    let version = board.runs.iter().find(ran)?.fields().plan_version;
    let planned =
        |plan: &&Plan| (&plan.fields().task, plan.fields().version) == (&end.task, version);
    board.plans.iter().find(planned).cloned()
}

impl BoardOperationsOver for ServedOperations {
    fn prepare(
        &self,
        request: Option<&BoardRequest>,
        sent: &ServedRun,
        reads: &dyn ProjectBoardStore,
    ) -> Result<OperationsOver, ServeError> {
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
        // Only a success asks, for its plan's items' evidence alone.
        let success = ended.filter(|end| end.outcome == RunEndOutcome::Succeeded);
        let plan = success.and_then(|end| plan_of(reads, end));
        let named = facts
            .zip(plan)
            .map(|(facts, plan)| facts.named_commits(&plan));
        let (budget, reading) = (self.read_budget, &self.revision_reads);
        let revisions = Arc::new(match named.unwrap_or(Ok(vec![])) {
            Ok(named) => revisions_read(&self.revisions, named, budget, reading)?,
            Err(why) => StatedRemoteRevisions::unread(&why),
        });
        let run = ended.map(|end| (&end.task, end.run));
        let liveness = facts.zip(run).and_then(|(facts, run)| facts.liveness(run));
        let reports = facts.zip(run).and_then(|(facts, run)| facts.reports(run));
        let undo = match sent {
            ServedRun::Undo(undo) => Some(undo.clone()),
            ServedRun::Plain | ServedRun::Facts(_) => None,
        };
        let host = Arc::clone(&self.clock);
        let sync = Arc::clone(&self.sync);
        Ok(Box::new(move |store| {
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
        }))
    }
}

/// Which of `named` the remote holds, read on a thread of its own within
/// `budget`: past it, none is (the end is refused, as for a remote that
/// cannot be read), and the read is left to finish on its own, each of its
/// git calls bounded. Nothing named, nothing is read; one read at a time
/// (`reading`), so reads past their budget never pile up.
///
/// # Errors
/// A read still runs: the write is busy (nothing written; its retry, the
/// same write id, is safe).
fn revisions_read(
    remote: &Arc<dyn RemoteRevisions>,
    named: Vec<String>,
    budget: Duration,
    reading: &Arc<AtomicBool>,
) -> Result<StatedRemoteRevisions, ServeError> {
    if named.is_empty() {
        return Ok(StatedRemoteRevisions::unread(
            "no revision was read before this write",
        ));
    }
    match reading.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst) {
        Ok(_) => {}
        Err(_) => return Err(ServeError::Reading),
    }
    /// Lets the next read go when the read ends, however it ends.
    struct Reading(Arc<AtomicBool>);
    impl Drop for Reading {
        fn drop(&mut self) {
            self.0.store(false, Ordering::SeqCst);
        }
    }
    let (remote, (sent, read)) = (Arc::clone(remote), std::sync::mpsc::channel());
    let reading = Reading(Arc::clone(reading));
    std::thread::spawn(move || {
        let _reading = reading;
        let _ = sent.send(StatedRemoteRevisions::read(&*remote, &named));
    });
    Ok(read.recv_timeout(budget).unwrap_or_else(|_| {
        StatedRemoteRevisions::unread("the remote was not read within its time budget")
    }))
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

#[cfg(test)]
#[path = "board_feed_tests.rs"]
mod feed_tests;

#[cfg(test)]
#[path = "board_replicate_tests.rs"]
mod replicate_tests;

#[cfg(test)]
#[path = "board_follower_tests.rs"]
mod follower_tests;
