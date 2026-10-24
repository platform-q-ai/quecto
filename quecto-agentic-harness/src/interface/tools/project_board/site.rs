//! Where a front end's board is, and the one way a request reaches it
//! (#2477): the `quecto board` CLI and the agent `board` tool name a
//! repository and its board remote, composition builds the board's
//! operations for what a request does (a read needs no committer), and
//! [`answer`] reads, builds and runs it, refusing a request before anything
//! is built.
use std::path::PathBuf;

use serde_json::Value;

use super::answer::FrontEndRefusal;
use super::recording::BoardCall;
use super::request::{RequestError, op_of, parse};
use super::{read, run_recorded_with, summaries_refusal};
use crate::application::project_board::operations::ListSummaries;
use crate::application::project_board::ports::{
    BoardOperations, CoordinatorLaunch, RunLiveness, StartedRuns,
};
use crate::domain::project_board::value_objects::telemetry::{BoardId, FaultKind, FrontEnd};

/// What a request does with the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Reads only: no committer is needed.
    Read,
    /// Writes: the repository's configured committer writes.
    Write,
}

/// What composition is given to build the board's operations.
pub struct ProjectBoardEnvironment {
    pub repo: PathBuf,
    pub remote: String,
    pub access: Access,
    /// The process environment, which composition allowlists for git.
    pub vars: Vec<(std::ffi::OsString, std::ffi::OsString)>,
    /// Set when the call the request came from was cancelled: the board
    /// then pushes nothing more (`None`: the request runs to its end).
    pub cancelled: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// What launches a started task's coordinator, and sees whether a
    /// run's swarm still runs: the agent tool's (S4c), which launches only
    /// while its session's policy allows spawn (it asks before a start and
    /// as it launches); `None` for a front end that starts nothing and sees
    /// no swarm (the CLI), so a start is refused and an end_run sees none
    /// running.
    pub launcher: Option<Launcher>,
    /// The session whose ended swarms' boards a run's end is recorded from
    /// (#2481 S4c follow-up): the agent tool's; `None` for a front end that
    /// keeps none (the CLI), whose ends record nothing reported.
    pub swarm_session: Option<String>,
}

/// A session's swarms as a front end reaches them (S4c): it launches a
/// started task's coordinator, sees whether a run's still runs, and names
/// the runs it started (#2655).
pub trait SessionSwarms: CoordinatorLaunch + RunLiveness + StartedRuns {}

impl<T: CoordinatorLaunch + RunLiveness + StartedRuns> SessionSwarms for T {}

/// A front end's coordinator launcher (S4c).
pub type Launcher = std::sync::Arc<dyn SessionSwarms>;

/// The board's operations, built by composition: the port the front ends
/// hand each request to (#2705 B6).
pub struct ProjectBoardHandles {
    pub operations: Box<dyn BoardOperations>,
    /// The board's id for its records (#2663): `None` where its remote
    /// gives none.
    pub board: Option<BoardId>,
}

/// Why composition could not build the board's operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildError {
    /// A write, with no committer configured.
    NoCommitter(String),
    /// The remote named is none the repository configures (#2659).
    NoSuchRemote(String),
    Unavailable(String),
}

pub type ProjectBoardHandlesBuilder =
    fn(&ProjectBoardEnvironment) -> Result<ProjectBoardHandles, BuildError>;

/// A front end's board: its repository and board remote, how composition
/// builds its operations (`None` when it is not composed), and what
/// launches a started task's coordinator (`None`: it starts nothing).
#[derive(Clone)]
pub struct BoardSite {
    pub repo: PathBuf,
    pub remote: String,
    pub build: Option<ProjectBoardHandlesBuilder>,
    pub launcher: Option<Launcher>,
    /// The session whose ended swarms' boards a run's end reads (`None`:
    /// none, as from the CLI).
    pub swarm_session: Option<String>,
}

impl BoardSite {
    /// The agent tool's board (S4c): on `repo`'s `origin`, built by
    /// `build`, starting tasks through `launcher`, its run ends read from
    /// the boards the teardown keeps under the session of `spawned_in`,
    /// the environment registry its coordinators are spawned into (#2481
    /// S4c follow-up).
    pub fn agent(
        repo: PathBuf,
        build: ProjectBoardHandlesBuilder,
        launcher: Launcher,
        spawned_in: &crate::domain::environments::entities::environment_registry::EnvironmentRegistry,
    ) -> Self {
        // The one authority on the session's board repository, which its
        // teardown judges a succeeded board run's completion against (#2666).
        crate::infrastructure::tools::agents::coordinator_launch::register_board_repository(
            spawned_in.session(),
            &repo,
        );
        Self {
            repo,
            remote: "origin".into(),
            build: Some(build),
            launcher: Some(launcher),
            swarm_session: Some(spawned_in.session().to_owned()),
        }
    }
}

impl std::fmt::Debug for BoardSite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoardSite")
            .field("repo", &self.repo)
            .field("remote", &self.remote)
            .field("build", &self.build.is_some())
            .field("launcher", &self.launcher.is_some())
            .field("swarm_session", &self.swarm_session)
            .finish()
    }
}

/// The answer to the request `text` on `site`'s board, noted on `call`;
/// once `cancelled` is set, the board pushes nothing more. Blocking: the
/// board's git and host reads hold the calling thread.
pub fn answer(
    site: &BoardSite,
    text: &str,
    cancelled: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    call: &mut BoardCall,
) -> Value {
    answer_with(site, text, cancelled, ListSummaries::Omitted, call)
}

/// [`answer`], a listing summarising its tasks as `summaries` asks (the
/// CLI's `--summary`); summaries asked of a request that lists nothing
/// are refused before anything is built.
pub fn answer_with(
    site: &BoardSite,
    text: &str,
    cancelled: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    summaries: ListSummaries,
    call: &mut BoardCall,
) -> Value {
    let value = match read(text) {
        Ok(value) => value,
        Err(error) => {
            let op = super::answer::start_steps::start_in_unread(text);
            call.named(op);
            return call.refuse_request(op, &error);
        }
    };
    // Read first: a request refused here builds nothing.
    let op = op_of(&value);
    call.named(op.as_deref());
    let parsed = match parse(&value) {
        Ok(parsed) => parsed,
        Err(error) => return call.refuse_request(op.as_deref(), &error),
    };
    call.parsed(&parsed);
    if let Some(refusal) = summaries_refusal(&parsed, summaries) {
        return call.refuse_request(op.as_deref(), &refusal);
    }
    let Some(build) = site.build else {
        return call.refuse_as_front_end(
            op.as_deref(),
            FrontEndRefusal::StoreUnavailable,
            "the project board capability is not composed",
        );
    };
    let access = match parsed.writes() {
        true => Access::Write,
        false => Access::Read,
    };
    let environment = environment_of(site, access, cancelled);
    match call.built(|| build(&environment)) {
        // The one entry point every front end's request goes through.
        Ok(handles) => {
            call.on_board(handles.board.as_ref());
            run_recorded_with(&*handles.operations, text, summaries, call)
        }
        Err(BuildError::NoCommitter(why)) => {
            call.refuse_as_front_end(op.as_deref(), FrontEndRefusal::NoCommitter, &why)
        }
        // A remote the clone lacks: on the command line, which names it
        // (`--remote`), the request's mistake; the agent tool names none
        // (its board is `origin`), so there the store is out of reach.
        Err(BuildError::NoSuchRemote(why)) => match call.front_end() {
            FrontEnd::Cli => {
                let error = RequestError::of("remote", why, FaultKind::InvalidValue);
                call.refuse_request(op.as_deref(), &error)
            }
            FrontEnd::Tool => {
                call.refuse_as_front_end(op.as_deref(), FrontEndRefusal::StoreUnavailable, &why)
            }
        },
        Err(BuildError::Unavailable(why)) => {
            call.refuse_as_front_end(op.as_deref(), FrontEndRefusal::StoreUnavailable, &why)
        }
    }
}

/// What composition is given to build `site`'s board for `access`.
fn environment_of(
    site: &BoardSite,
    access: Access,
    cancelled: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> ProjectBoardEnvironment {
    ProjectBoardEnvironment {
        repo: site.repo.clone(),
        remote: site.remote.clone(),
        access,
        vars: std::env::vars_os().collect(),
        cancelled,
        launcher: site.launcher.clone(),
        swarm_session: site.swarm_session.clone(),
    }
}

/// The ends `site`'s session owes as it ends (#2655): each run its starts
/// launched that the board still shows open. None when the board is not
/// composed; why, when it cannot be read.
///
/// # Errors
/// The board could not be built or read: what it said.
pub fn left_runs(
    site: &BoardSite,
) -> Result<Vec<crate::application::project_board::dto::RunEnd>, String> {
    match site.launcher.as_ref() {
        Some(launcher) if !launcher.started().is_empty() => {}
        _ => return Ok(vec![]),
    }
    let Some(build) = site.build else {
        return Ok(vec![]);
    };
    match build(&environment_of(site, Access::Read, None)) {
        Ok(handles) => handles
            .operations
            .left_runs()
            .map_err(|error| error.to_string()),
        Err(
            BuildError::NoCommitter(why)
            | BuildError::NoSuchRemote(why)
            | BuildError::Unavailable(why),
        ) => Err(why),
    }
}
