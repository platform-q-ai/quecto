//! The board's operations on a board server (epic #2705 B7): picked when
//! one is named, by the front end's `--url`, else `QUECTO_BOARD_URL`,
//! else the global config's `board.url`. Its token, for TCP, comes from
//! `QUECTO_BOARD_TOKEN`, else `board.token`, never from the command line.
//! Nothing here reads the board or the clock: a write is the one request
//! the server answers. git is asked only what the front end does not
//! know: the board's id, and for a write its committer
//! (`QUECTO_BOARD_COMMITTER` first).
use std::ffi::OsString;
use std::path::PathBuf;

use serde_json::Value;

use super::project_board::{BoardLocation, board_committer, store_reason, write_refusal};
use crate::infrastructure::http::board_remote::{RemoteBoardOperations, endpoint_of};
use crate::infrastructure::persistence::project_board::git::{Git, GitEnvironment};
use crate::infrastructure::persistence::project_board::store::{GitBoardStore, StoreOpenError};
use crate::interface::cli::controllers::board::{
    Access, BuildError, ProjectBoardEnvironment, ProjectBoardHandles,
};

/// The variable naming a board server for every front end.
pub const URL_VARIABLE: &str = "QUECTO_BOARD_URL";
/// The variable holding the device's token for a board server over TCP.
pub const TOKEN_VARIABLE: &str = "QUECTO_BOARD_TOKEN";

/// The board server `environment` names, if any: its front end's, its
/// variable's, or its global config's.
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
/// A URL or token refused, no board id to be had, or for a write no
/// committer.
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
    let operations = RemoteBoardOperations::new(vec![client], board.clone(), committer);
    Ok(ProjectBoardHandles {
        operations: Box::new(operations),
        board: Some(board),
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

/// `board.<key>` in the global config (`$QUECTO_BASE_DIR/config.json`,
/// else `~/.quecto/config.json`), when it is text; a config that cannot
/// be read names none, so it never stops the board on this clone.
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
