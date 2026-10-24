//! `quecto board-server serve` (epic #2705 B4): the board over HTTP/1.1 on
//! a Unix socket, to this user only, until SIGINT or SIGTERM. Composition
//! builds the graph ([`BoardServerBuilder`]); this owns the command line,
//! the route table and the one tokio runtime.
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use crate::application::project_board::use_cases::ServeBoard;
use crate::infrastructure::http::board_service::listener::PeerCredentials;
use crate::interface::cli::CliContext;

pub mod routes;
pub mod serve;

/// What composition builds a board server from (the environment is
/// allowlisted for git there).
#[derive(Debug, Clone)]
pub struct BoardServerEnvironment {
    pub repo: PathBuf,
    pub remote: String,
    pub vars: Vec<(OsString, OsString)>,
}

/// The board server's graph: its reads, peer reader and the uid let in.
pub struct BoardServerHandles {
    pub serve: ServeBoard,
    pub peers: Arc<dyn PeerCredentials>,
    pub uid: u32,
}

/// Composition's builder of a board server.
pub type BoardServerBuilder = fn(&BoardServerEnvironment) -> Result<BoardServerHandles, String>;

/// `quecto board-server <args>`: 0 once stopped by a signal, 1 when it
/// could not start or serve; its one line out comes as soon as it serves.
pub fn cmd_board_server(
    ctx: &CliContext,
    args: &[String],
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    let began = std::time::Instant::now();
    let options = match serve::parse(args) {
        Ok(options) => options,
        Err(error) => {
            stderr.push_str(&format!("{error}\n{}\n", serve::USAGE));
            return 1;
        }
    };
    let Some(build) = ctx.board_server else {
        stderr.push_str("board-server: the board server is not composed\n");
        return 1;
    };
    let environment = BoardServerEnvironment {
        repo: options.repo,
        remote: options.remote,
        vars: std::env::vars_os().collect(),
    };
    let handles = match build(&environment) {
        Ok(handles) => handles,
        Err(error) => {
            stderr.push_str(&format!("{error}\n"));
            return 1;
        }
    };
    let live = ctx.live_output;
    let served = serve::serve(
        handles,
        &options.socket,
        began,
        serve::signalled,
        |started| {
            let line = format!(
                "board-server: serving board {} on {} (ready in {} ms)\n",
                started.board.as_str(),
                started.socket.display(),
                started.elapsed.as_millis()
            );
            match live {
                // A server runs until stopped: its line cannot wait for
                // the command's end.
                true => {
                    print!("{line}");
                    let _ = std::io::Write::flush(&mut std::io::stdout());
                }
                false => stdout.push_str(&line),
            }
        },
    );
    match served {
        Ok(()) => 0,
        Err(error) => {
            stderr.push_str(&format!("{error}\n"));
            1
        }
    }
}
