//! `quecto board-server serve --socket <path> --repo <clone> [--remote
//! <name>]` (epic #2705 B4): serves the repository's project board over
//! HTTP/1.1 on a Unix socket, until SIGINT or SIGTERM. Reads only, for
//! now: `GET /v1/status`, `GET /v1/time` and `GET
//! /v1/boards/{id}/snapshot`. Only a process of the server's own user may
//! connect (its peer credentials are read on accept); the socket is
//! created mode 0600 in a directory of mode 0700.
//!
//! Composition builds the board's store, clock and file codec
//! ([`BoardServerBuilder`]); this front end owns the command line, the
//! route table and the one tokio runtime the command starts.
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use crate::application::project_board::use_cases::ServeBoard;
use crate::infrastructure::http::board_service::listener::PeerCredentials;

pub mod routes;
pub mod serve;

/// What composition builds a board server from: the clone, its board
/// remote, and the process environment (allowlisted for git there).
#[derive(Debug, Clone)]
pub struct BoardServerEnvironment {
    pub repo: PathBuf,
    pub remote: String,
    pub vars: Vec<(OsString, OsString)>,
}

/// The board server's graph, as composition builds it: the reads it
/// answers, the peer-credential reader, and the one uid let in.
pub struct BoardServerHandles {
    pub serve: ServeBoard,
    pub peers: Arc<dyn PeerCredentials>,
    pub uid: u32,
}

/// Composition's builder of a board server.
pub type BoardServerBuilder = fn(&BoardServerEnvironment) -> Result<BoardServerHandles, String>;
