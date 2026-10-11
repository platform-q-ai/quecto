//! The board operations as their front ends send and answer them (#2477),
//! shared by the `quecto board` CLI and the agent `board` tool (S2b): a
//! request is one JSON object read by [`request`] (the command line builds
//! the same object, [`command_line`]), run by the application's
//! [`BoardOperations`], and answered in one stable JSON shape by
//! [`answer`]. One path, so the front ends cannot drift apart.
pub mod answer;
pub mod command_line;
pub mod json;
pub mod request;
pub mod tool;

use std::path::PathBuf;

use serde_json::Value;

use crate::application::project_board::operations::BoardOperations;
use answer::ErrorCode;

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
}

/// The board's operations, built by composition.
pub struct ProjectBoardHandles {
    pub operations: BoardOperations,
}

/// Why composition could not build the board's operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildError {
    /// A write, with no committer configured.
    NoCommitter(String),
    Unavailable(String),
}

/// Composition's builder of the board's operations for one request. It
/// blocks (git, the network): front ends call it off the async runtime.
pub type ProjectBoardHandlesBuilder =
    fn(&ProjectBoardEnvironment) -> Result<ProjectBoardHandles, BuildError>;

/// The remote a board is on unless a front end names another.
pub const DEFAULT_REMOTE: &str = "origin";

/// Where a front end's board is: the repository and its board remote.
pub struct BoardLocation {
    pub repo: PathBuf,
    pub remote: String,
}

/// The answer to the request `value` on the board at `location`, built
/// by `build` (`None` when composition gave none): the request is read
/// first (a refused one builds nothing), then the board is built for the
/// access the request needs, then run. Both front ends answer through
/// here. It blocks: the tool calls it off the async runtime.
pub fn serve(
    build: Option<ProjectBoardHandlesBuilder>,
    location: BoardLocation,
    value: &Value,
) -> Value {
    let op = request::op_of(value);
    let parsed = match request::parse(value) {
        Ok(parsed) => parsed,
        Err(error) => return answer::request_refused(op.as_deref(), &error),
    };
    let Some(build) = build else {
        return answer::reason_refused(
            op.as_deref(),
            ErrorCode::StoreUnavailable,
            "the project board capability is not composed",
        );
    };
    let environment = ProjectBoardEnvironment {
        repo: location.repo,
        remote: location.remote,
        access: match parsed.writes() {
            true => Access::Write,
            false => Access::Read,
        },
        vars: std::env::vars_os().collect(),
    };
    match build(&environment) {
        Ok(handles) => run(&handles.operations, value),
        Err(BuildError::NoCommitter(why)) => {
            answer::reason_refused(op.as_deref(), ErrorCode::NoCommitter, &why)
        }
        Err(BuildError::Unavailable(why)) => {
            answer::reason_refused(op.as_deref(), ErrorCode::StoreUnavailable, &why)
        }
    }
}

/// The answer to the request `value`.
pub fn run(operations: &BoardOperations, value: &Value) -> Value {
    let op = request::op_of(value);
    let request = match request::parse(value) {
        Ok(request) => request,
        Err(error) => return answer::request_refused(op.as_deref(), &error),
    };
    let op = op.expect("a parsed request names an op");
    match operations.execute(&request) {
        Ok(outcome) => answer::outcome(&op, &outcome, operations.clock_note(&request)),
        Err(error) => answer::board_refused(&op, &error),
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "boards_tests.rs"]
pub(crate) mod boards;
