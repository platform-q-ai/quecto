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
use super::request::{op_of, parse};
use super::{read, run_recorded};
use crate::application::project_board::ports::{CoordinatorLaunch, RunLiveness};
use crate::application::project_board::use_cases::RunBoardOperation;

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
}

/// A session's swarms as a front end reaches them (S4c): it launches a
/// started task's coordinator, and sees whether a run's still runs.
pub trait SessionSwarms: CoordinatorLaunch + RunLiveness {}

impl<T: CoordinatorLaunch + RunLiveness> SessionSwarms for T {}

/// A front end's coordinator launcher (S4c).
pub type Launcher = std::sync::Arc<dyn SessionSwarms>;

/// The board's operations, built by composition.
pub struct ProjectBoardHandles {
    pub operations: RunBoardOperation,
}

/// Why composition could not build the board's operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildError {
    /// A write, with no committer configured.
    NoCommitter(String),
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
}

impl std::fmt::Debug for BoardSite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoardSite")
            .field("repo", &self.repo)
            .field("remote", &self.remote)
            .field("build", &self.build.is_some())
            .field("launcher", &self.launcher.is_some())
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
    let value = match read(text) {
        Ok(value) => value,
        Err(error) => return call.refuse_request(None, &error),
    };
    // Read first: a request refused here builds nothing.
    let op = op_of(&value);
    call.named(op.as_deref());
    let parsed = match parse(&value) {
        Ok(parsed) => parsed,
        Err(error) => return call.refuse_request(op.as_deref(), &error),
    };
    call.parsed(&parsed);
    let Some(build) = site.build else {
        return call.refuse_as_front_end(
            op.as_deref(),
            FrontEndRefusal::StoreUnavailable,
            "the project board capability is not composed",
        );
    };
    let environment = ProjectBoardEnvironment {
        repo: site.repo.clone(),
        remote: site.remote.clone(),
        access: match parsed.writes() {
            true => Access::Write,
            false => Access::Read,
        },
        vars: std::env::vars_os().collect(),
        cancelled,
        launcher: site.launcher.clone(),
    };
    match call.built(|| build(&environment)) {
        // The one entry point every front end's request goes through.
        Ok(handles) => run_recorded(&handles.operations, text, call),
        Err(BuildError::NoCommitter(why)) => {
            call.refuse_as_front_end(op.as_deref(), FrontEndRefusal::NoCommitter, &why)
        }
        Err(BuildError::Unavailable(why)) => {
            call.refuse_as_front_end(op.as_deref(), FrontEndRefusal::StoreUnavailable, &why)
        }
    }
}
