//! `quecto board-server` (epic #2705 B4, B5): `serve` gives the board over
//! HTTP/1.1 on a Unix socket (this user only) and allowlisted TCP (devices
//! with a token) until SIGINT or SIGTERM; `token` administers the tokens.
//! Composition builds the graph ([`BoardServerBuilder`]).
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use crate::application::project_board::ports::ListenAddress;
use crate::application::project_board::use_cases::{
    BoardAccess, FollowBoard, ReplicateBoard, ServeBoard, ServeBoardOperation, ServeReplication,
};
use crate::infrastructure::http::board_service::listener::PeerCredentials;
use crate::interface::cli::CliContext;

pub mod backup;
pub mod feed;
pub mod fence;
pub mod history;
mod op;
pub mod routes;
pub mod serve;
pub mod token;

/// What composition builds a board server from (the environment is
/// allowlisted for git there).
#[derive(Debug, Clone)]
pub struct BoardServerEnvironment {
    pub repo: PathBuf,
    pub remote: String,
    pub vars: Vec<(OsString, OsString)>,
    /// The global config, whose `board_server.listen` names TCP addresses.
    pub config: PathBuf,
    /// The tokens' and the board's directory: `--state`, else
    /// `<base_dir>/board-server/<board id>`.
    pub state: Option<PathBuf>,
    pub base_dir: PathBuf,
    /// The primary this server follows (B11); none: it is a primary.
    pub follow: Option<FollowPrimary>,
}

/// A follower's primary (B11): its URL, and the file its device token is
/// read from (never the command line), for a primary over TCP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowPrimary {
    pub url: String,
    pub token_file: Option<PathBuf>,
}

/// The board server's graph, and the TCP addresses its config names.
pub struct BoardServerHandles {
    pub serve: ServeBoard,
    /// The board's operations (B7).
    pub operations: ServeBoardOperation,
    /// The board's change feed (B10); none answers its routes 404.
    pub feed: Option<FollowBoard>,
    /// The batches it gives its followers (B11).
    pub replication: Option<ServeReplication>,
    /// A follower's replication of its primary, and the primary it names
    /// in a write's 409 (B11); none on a primary.
    pub follower: Option<(Arc<ReplicateBoard>, String)>,
    pub access: BoardAccess,
    pub peers: Arc<dyn PeerCredentials>,
    pub uid: u32,
    pub listen: Vec<ListenAddress>,
}

/// Composition's builder of a board server.
pub type BoardServerBuilder = fn(&BoardServerEnvironment) -> Result<BoardServerHandles, String>;

/// `quecto board-server <args>`: 0 once `serve` is stopped by a signal or
/// `token` did as asked; 1 when either could not.
pub fn cmd_board_server(
    ctx: &CliContext,
    args: &[String],
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    if let Some(("export", rest)) = args
        .split_first()
        .map(|(first, rest)| (first.as_str(), rest))
    {
        return history::run(ctx, rest, stdout, stderr);
    }
    if let Some(("import", rest)) = args
        .split_first()
        .map(|(first, rest)| (first.as_str(), rest))
    {
        return history::run_import(ctx, rest, stdout, stderr);
    }
    if let Some(("backup", rest)) = args
        .split_first()
        .map(|(first, rest)| (first.as_str(), rest))
    {
        return backup::run(ctx, rest, stdout, stderr);
    }
    if let Some(("fence", rest)) = args
        .split_first()
        .map(|(first, rest)| (first.as_str(), rest))
    {
        return fence::run(ctx, rest, stdout, stderr);
    }
    let began = std::time::Instant::now();
    if let Some(("token", rest)) = args
        .split_first()
        .map(|(first, rest)| (first.as_str(), rest))
    {
        return match token::parse(rest) {
            Ok(options) => token::run(&options, stdout, stderr),
            Err(error) => {
                stderr.push_str(&format!("{error}\n{}\n", serve::USAGE));
                1
            }
        };
    }
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
    let base_dir = ctx.base_dir();
    let environment = BoardServerEnvironment {
        repo: options.repo,
        remote: options.remote,
        vars: std::env::vars_os().collect(),
        config: ctx
            .config_path
            .clone()
            .unwrap_or_else(|| base_dir.join("config.json")),
        state: options.state,
        base_dir,
        follow: options.follow,
    };
    let handles = match build(&environment) {
        Ok(handles) => handles,
        Err(error) => {
            stderr.push_str(&format!("{error}\n"));
            return 1;
        }
    };
    // `--listen` replaces the config's addresses; with neither, the socket alone.
    let listen = match options.listen.is_empty() {
        true => handles.listen.clone(),
        false => options.listen,
    };
    let live = ctx.live_output;
    let following = handles
        .follower
        .as_ref()
        .map_or(String::new(), |(_, primary)| {
            format!(", a read-only follower of {primary}")
        });
    let served = serve::serve(
        handles,
        &options.socket,
        &listen,
        began,
        serve::signalled,
        |started| {
            let tcp: String = started
                .tcp
                .iter()
                .map(|at| format!(" and tcp {at}"))
                .collect();
            let line = format!(
                "board-server: serving board {} on {}{tcp}{following} (ready in {} ms)\n",
                started.board.as_str(),
                started.socket.display(),
                started.elapsed.as_millis()
            );
            // A server runs until stopped: its line cannot wait for the
            // command's end.
            match live {
                true => serve::announce(&mut std::io::stdout().lock(), &line),
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
