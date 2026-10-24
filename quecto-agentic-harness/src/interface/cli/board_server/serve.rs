//! Red stub (B4): `quecto board-server serve`.
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::BoardServerHandles;
use crate::domain::project_board::value_objects::telemetry::BoardId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServeOptions {
    pub socket: PathBuf,
    pub repo: PathBuf,
    pub remote: String,
}

/// # Errors
/// Not yet parsed.
pub fn parse(args: &[String]) -> Result<ServeOptions, String> {
    Err(format!(
        "board-server: {} arguments not yet parsed",
        args.len()
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Started {
    pub socket: PathBuf,
    pub board: BoardId,
    pub elapsed: Duration,
}

/// # Errors
/// Not yet served.
pub fn serve<S, F>(
    handles: BoardServerHandles,
    socket: &Path,
    began: Instant,
    stop: S,
    ready: impl FnOnce(&Started),
) -> Result<(), String>
where
    S: FnOnce() -> F,
    F: Future<Output = ()>,
{
    drop((handles, began, stop, ready));
    Err(format!("board-server: {} not yet served", socket.display()))
}

#[cfg(test)]
#[path = "serve_tests.rs"]
mod tests;
