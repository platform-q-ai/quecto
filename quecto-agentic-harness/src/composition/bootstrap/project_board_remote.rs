//! The board's operations on a board server (epic #2705 B7), named by
//! `--url`, `QUECTO_BOARD_URL` or `board.url`, its token never from argv;
//! git is asked only the board's id when not given, and a write's committer.
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;

use super::project_board::{
    BoardLocation, board_committer, kept_reports, store_reason, write_refusal,
};
use super::project_board_remote_runs::{RemoteRuns, RunSwarms};
use crate::application::project_board::ports::{CoordinatorLaunch, RunLiveness, StartedRuns};
use crate::application::project_board::use_cases::LaunchBegunRun;
use crate::application::project_board::use_cases::{WatchBoard, WatchTiming};
use crate::infrastructure::http::board_remote::{RemoteBoardOperations, endpoint_of};
use crate::infrastructure::http::board_remote_runs::RemoteBoardRuns;
use crate::infrastructure::http::board_watch::{
    RemoteBoardStreams, ThreadPauses, stdout_has_reader,
};
use crate::infrastructure::persistence::project_board::git::{Git, GitEnvironment};
use crate::infrastructure::persistence::project_board::store::{GitBoardStore, StoreOpenError};
use crate::interface::cli::controllers::board::watch::BoardWatchHandles;
use crate::interface::cli::controllers::board::{
    Access, BuildError, ProjectBoardEnvironment, ProjectBoardHandles,
};

pub const URL_VARIABLE: &str = "QUECTO_BOARD_URL";
pub const TOKEN_VARIABLE: &str = "QUECTO_BOARD_TOKEN";

/// The board server `environment` names, if any.
pub fn server_url(environment: &ProjectBoardEnvironment) -> Option<String> {
    environment
        .known
        .url
        .clone()
        .or_else(|| var(&environment.vars, URL_VARIABLE))
        .or_else(|| configured(&environment.vars, "url"))
}

/// The board's operations on the server at `url`.
///
/// # Errors
/// A URL or token refused, no board id, or for a write no committer.
pub fn remote_handles(
    environment: &ProjectBoardEnvironment,
    url: &str,
) -> Result<ProjectBoardHandles, BuildError> {
    let unavailable = BuildError::Unavailable;
    let token =
        var(&environment.vars, TOKEN_VARIABLE).or_else(|| configured(&environment.vars, "token"));
    let client = endpoint_of(url, token.as_deref())
        .map_err(|why| unavailable(format!("the board server: {why}")))?;
    let vars = GitEnvironment::allowlisted(environment.vars.iter().cloned());
    let committer = match (environment.access, board_committer(&environment.vars)) {
        (Access::Write, Some(named)) => Some(named.map_err(BuildError::NoCommitter)?),
        (Access::Write, None) => {
            Some(Git::configured_committer(&environment.repo, &vars).map_err(write_refusal)?)
        }
        (Access::Read, _) => None,
    };
    let board = match &environment.known.board_id {
        Some(board) => board.clone(),
        None => located(environment, vars)?,
    };
    let remote = Arc::new(RemoteBoardOperations::new(
        vec![client],
        board.clone(),
        committer,
    ));
    // A start is begun there and launched here, undone there on failure;
    // an end carries what this session's swarms saw of its run (B8).
    let swarms = environment.launcher.clone();
    let launch = LaunchBegunRun::new(
        Arc::new(RemoteBoardRuns(Arc::clone(&remote))),
        swarms
            .clone()
            .map(|swarms| swarms as Arc<dyn CoordinatorLaunch>),
        environment.cancelled.clone(),
    );
    let liveness = swarms.clone().map(|swarms| swarms as Arc<dyn RunLiveness>);
    let swarms = RunSwarms {
        started: swarms.map(|swarms| swarms as Arc<dyn StartedRuns>),
        liveness: liveness.clone(),
        reports: kept_reports(environment),
    };
    let operations = RemoteRuns::new(remote, launch.seeing(liveness), swarms);
    Ok(ProjectBoardHandles {
        operations: Box::new(operations),
        board: Some(board),
    })
}

/// A watch of the change feed of the server `environment` names (B10):
/// its URL and token as for its operations; no committer. Its output is
/// open while standard output has a reader.
///
/// # Errors
/// No board server named, a URL or token refused, or no board id.
pub fn build_board_watch(
    environment: &ProjectBoardEnvironment,
) -> Result<BoardWatchHandles, BuildError> {
    let unavailable = BuildError::Unavailable;
    let url = server_url(environment).ok_or_else(|| {
        unavailable(format!(
            "watch needs a board server: give --url, or set {URL_VARIABLE} or board.url"
        ))
    })?;
    let token =
        var(&environment.vars, TOKEN_VARIABLE).or_else(|| configured(&environment.vars, "token"));
    let client = endpoint_of(&url, token.as_deref())
        .map_err(|why| unavailable(format!("the board server: {why}")))?;
    let board = match &environment.known.board_id {
        Some(board) => board.clone(),
        None => located(
            environment,
            GitEnvironment::allowlisted(environment.vars.iter().cloned()),
        )?,
    };
    let streams = Box::new(RemoteBoardStreams::new(client, board));
    let watch = WatchBoard::new(streams, Arc::new(ThreadPauses), WatchTiming::PRODUCTION);
    Ok(BoardWatchHandles {
        watch,
        output_open: stdout_has_reader,
    })
}

/// The board's id as git makes it from the clone's remote.
fn located(
    environment: &ProjectBoardEnvironment,
    vars: GitEnvironment,
) -> Result<crate::domain::project_board::value_objects::telemetry::BoardId, BuildError> {
    let unavailable = BuildError::Unavailable;
    let git = Git::reader(&environment.repo, vars)
        .map_err(|error| unavailable(format!("git cannot run here: {error}")))?;
    let store = GitBoardStore::new(git, &environment.remote).map_err(|error| match error {
        StoreOpenError::NoSuchRemote(why) => BuildError::NoSuchRemote(why),
        StoreOpenError::Unavailable(error) => unavailable(store_reason(&error)),
    })?;
    let location = BoardLocation::resolve(&environment.known, &store, &environment.repo)
        .map_err(|error| unavailable(store_reason(&error)))?;
    location
        .board
        .ok_or_else(|| unavailable("the board server needs the board's id: give --board-id".into()))
}

/// The variable `name` in `vars`, when it is set to text.
fn var(vars: &[(OsString, OsString)], name: &str) -> Option<String> {
    vars.iter()
        .find(|(key, _)| key == name)
        .and_then(|(_, value)| value.to_str())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// `board.<key>` in the global config, when it is text (unreadable: none).
fn configured(vars: &[(OsString, OsString)], key: &str) -> Option<String> {
    let base = var(vars, "QUECTO_BASE_DIR")
        .map(PathBuf::from)
        .or_else(|| var(vars, "HOME").map(|home| PathBuf::from(home).join(".quecto")))?;
    let bytes = std::fs::read(base.join("config.json")).ok()?;
    let config: Value = serde_json::from_slice(&bytes).ok()?;
    config["board"][key]
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
#[path = "project_board_remote_tests.rs"]
mod tests;
