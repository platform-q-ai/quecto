//! `quecto board-server serve`: its options, and the one tokio runtime
//! the command starts, confined to it. The socket is bound before the
//! runtime starts and removed when the server stops.
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::BoardServerHandles;
use super::routes::BoardRoutes;
use crate::domain::project_board::value_objects::telemetry::BoardId;

use crate::infrastructure::http::board_service::listener::{self, ConnectionLimits, ServiceParts};

/// How the command is used.
pub const USAGE: &str =
    "usage: quecto board-server serve --socket <path> --repo <clone> [--remote <name>]";

/// The blocking threads the server's reads may hold at once.
const BLOCKING_THREADS: usize = 16;

/// `serve`'s options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServeOptions {
    pub socket: PathBuf,
    pub repo: PathBuf,
    pub remote: String,
}

/// `args` (after `board-server`) as `serve`'s options: `--socket` and
/// `--repo` required, `--remote` defaulting to `origin`, each once, and
/// nothing else.
///
/// # Errors
/// Another action, an option outside the allowlist, one without its
/// value or given twice, or a required one missing.
pub fn parse(args: &[String]) -> Result<ServeOptions, String> {
    let refused = |why: String| Err(format!("board-server: {why}"));
    let Some((action, options)) = args.split_first() else {
        return refused("an action is needed: serve".into());
    };
    match action.as_str() {
        "serve" => {}
        other => return refused(format!("unknown action '{other}': the action is serve")),
    }
    let (mut socket, mut repo, mut remote) = (None, None, None);
    let mut rest = options.iter();
    while let Some(option) = rest.next() {
        let slot = match option.as_str() {
            "--socket" => &mut socket,
            "--repo" => &mut repo,
            "--remote" => &mut remote,
            other => return refused(format!("unknown option '{other}'")),
        };
        let Some(value) = rest.next() else {
            return refused(format!("{option} needs a value"));
        };
        match slot.replace(value.clone()) {
            None => {}
            Some(_) => return refused(format!("{option} is given twice")),
        }
    }
    match (socket, repo) {
        (Some(socket), Some(repo)) => Ok(ServeOptions {
            socket: PathBuf::from(socket),
            repo: PathBuf::from(repo),
            remote: remote.unwrap_or_else(|| "origin".into()),
        }),
        (None, _) => refused("--socket <path> is required".into()),
        (Some(_), None) => refused("--repo <clone> is required".into()),
    }
}

/// A server that has started: where, which board, and how long it took
/// from the command's start to its socket listening.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Started {
    pub socket: PathBuf,
    pub board: BoardId,
    pub elapsed: Duration,
}

/// Serves `handles` on `socket` until the future `stop` makes completes:
/// binds the socket, starts the runtime, tells `ready`, serves, then
/// removes the socket.
///
/// # Errors
/// The socket could not be bound, the runtime not started, or the
/// listener not served.
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
    let board = handles.serve.board().clone();
    let bound = listener::bind(socket).map_err(|error| format!("board-server: {error}"))?;
    // The socket's identity: only it is removed at the end, never a newer
    // server's socket bound at the same path since.
    let ours = identity(socket);
    let parts = ServiceParts {
        handler: Arc::new(BoardRoutes::new(handles.serve)),
        peers: handles.peers,
        uid: handles.uid,
        limits: ConnectionLimits::default(),
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(BLOCKING_THREADS)
        .enable_all()
        .build();
    let runtime = match runtime {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = remove_socket(socket, ours);
            return Err(format!("board-server: the runtime did not start: {error}"));
        }
    };
    let served = runtime.block_on(async {
        // The stop is made inside the runtime (a signal watch needs it).
        let stop = stop();
        ready(&Started {
            socket: socket.to_path_buf(),
            board,
            elapsed: began.elapsed(),
        });
        listener::serve(bound, parts, stop).await
    });
    // Requests in flight have a moment to finish; then the server goes.
    runtime.shutdown_timeout(Duration::from_secs(1));
    let removed = remove_socket(socket, ours);
    served.map_err(|error| format!("board-server: {error}"))?;
    removed
}

/// The device and inode of the file at `path`.
fn identity(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::symlink_metadata(path).ok()?;
    Some((meta.dev(), meta.ino()))
}

/// Removes the server's own socket: only the file it bound, by `ours`.
fn remove_socket(socket: &Path, ours: Option<(u64, u64)>) -> Result<(), String> {
    match (identity(socket), ours) {
        (Some(found), Some(bound)) if found == bound => std::fs::remove_file(socket)
            .map_err(|error| format!("board-server: {}: {error}", socket.display())),
        (Some(_) | None, _) => Ok(()),
    }
}

/// Completes on SIGINT or SIGTERM. A watch that cannot be set completes
/// at once, saying why, so the server never runs unstoppable.
pub fn signalled() -> impl Future<Output = ()> {
    use tokio::signal::unix::{SignalKind, signal};
    let watches = signal(SignalKind::interrupt()).and_then(|interrupt| {
        signal(SignalKind::terminate()).map(|terminate| (interrupt, terminate))
    });
    async move {
        match watches {
            Ok((mut interrupt, mut terminate)) => {
                tokio::select! {
                    _ = interrupt.recv() => {}
                    _ = terminate.recv() => {}
                }
            }
            Err(error) => eprintln!("board-server: cannot watch SIGINT and SIGTERM: {error}"),
        }
    }
}

#[cfg(test)]
#[path = "serve_tests.rs"]
mod tests;
