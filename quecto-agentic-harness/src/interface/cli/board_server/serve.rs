//! `quecto board-server serve`: its options, and the one tokio runtime,
//! confined to the command.
use std::future::Future;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::BoardServerHandles;
use super::routes::BoardRoutes;
use crate::application::project_board::ports::ListenAddress;
use crate::domain::project_board::value_objects::telemetry::BoardId;
use crate::infrastructure::http::board_service::listener::{
    ConnectionLimits, Listener, ServiceParts, bind, bind_tcp, serve as listen,
};

pub const USAGE: &str = "usage: quecto board-server serve --socket <path> --repo <clone> \
     [--remote <name>] [--listen <ip:port>]... [--state <dir>]\n       \
     quecto board-server token add <name> | revoke <name> | list --socket <path>";

/// The blocking threads the server's reads may hold at once.
const BLOCKING_THREADS: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServeOptions {
    pub socket: PathBuf,
    pub repo: PathBuf,
    pub remote: String,
    /// TCP addresses, each on the allowlist; none leaves the config's.
    pub listen: Vec<ListenAddress>,
    /// Where the server keeps its tokens; none is the default.
    pub state: Option<PathBuf>,
}

/// `args` (after `board-server`) as `serve`'s options: `--listen` as
/// often as wanted, each other at most once.
/// # Errors
/// Another action or option, one without its value or twice, an address
/// off the allowlist, or one of `--socket` and `--repo` missing.
pub fn parse(args: &[String]) -> Result<ServeOptions, String> {
    let refused = |why: String| Err(format!("board-server: {why}"));
    let Some((action, options)) = args.split_first() else {
        return refused("an action is needed: serve or token".into());
    };
    match action.as_str() {
        "serve" => {}
        other => {
            return refused(format!(
                "unknown action '{other}': the actions are serve and token"
            ));
        }
    }
    let (mut socket, mut repo, mut remote, mut state) = (None, None, None, None);
    let mut listen = Vec::new();
    let mut rest = options.iter();
    while let Some(option) = rest.next() {
        let Some(value) = rest.next() else {
            return match option.as_str() {
                "--socket" | "--repo" | "--remote" | "--state" | "--listen" => {
                    refused(format!("{option} needs a value"))
                }
                other => refused(format!("unknown option '{other}'")),
            };
        };
        let slot = match option.as_str() {
            "--socket" => &mut socket,
            "--repo" => &mut repo,
            "--remote" => &mut remote,
            "--state" => &mut state,
            "--listen" => match ListenAddress::parse(value) {
                Ok(address) => {
                    listen.push(address);
                    continue;
                }
                Err(why) => return refused(format!("--listen {why}")),
            },
            other => return refused(format!("unknown option '{other}'")),
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
            listen,
            state: state.map(PathBuf::from),
        }),
        (None, _) => refused("--socket <path> is required".into()),
        (Some(_), None) => refused("--repo <clone> is required".into()),
    }
}

/// A server that has started, and how long it took to listen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Started {
    pub socket: PathBuf,
    /// The TCP addresses bound (a port 0 resolved).
    pub tcp: Vec<SocketAddr>,
    pub board: BoardId,
    pub elapsed: Duration,
}

/// Binds `socket` and each of `tcp`, starts the runtime, tells `ready`,
/// serves `handles` until the future `stop` makes completes, then removes
/// the socket.
/// # Errors
/// The socket or an address could not be bound, the runtime started, or
/// it served.
pub fn serve<S, F>(
    handles: BoardServerHandles,
    socket: &Path,
    tcp: &[ListenAddress],
    began: Instant,
    stop: S,
    ready: impl FnOnce(&Started),
) -> Result<(), String>
where
    S: FnOnce() -> F,
    F: Future<Output = ()>,
{
    let board = handles.serve.board().clone();
    let bound = bind(socket).map_err(|error| format!("board-server: {error}"))?;
    // Only this socket is removed at the end, never a newer server's.
    let ours = identity(socket);
    let gone = |why: String| {
        let _ = remove_socket(socket, ours);
        Err(format!("board-server: {why}"))
    };
    let mut listeners = vec![Listener::Unix(bound)];
    let mut addresses = Vec::new();
    for address in tcp {
        match bind_tcp(address.socket_addr()) {
            Ok((listener, bound)) => {
                listeners.push(Listener::Tcp(listener));
                addresses.push(bound);
            }
            Err(error) => return gone(error.to_string()),
        }
    }
    let parts = ServiceParts {
        handler: Arc::new(BoardRoutes::new(handles.serve, handles.access)),
        peers: handles.peers,
        uid: handles.uid,
        limits: ConnectionLimits::PRODUCTION,
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(BLOCKING_THREADS)
        .enable_all()
        .build();
    let runtime = match runtime {
        Ok(runtime) => runtime,
        Err(error) => return gone(format!("the runtime did not start: {error}")),
    };
    let served = runtime.block_on(async {
        // The stop is made inside the runtime (a signal watch needs it).
        let stop = stop();
        ready(&Started {
            socket: socket.to_path_buf(),
            tcp: addresses,
            board,
            elapsed: began.elapsed(),
        });
        listen(listeners, parts, stop).await
    });
    // Requests in flight have a moment to finish; then the server goes.
    runtime.shutdown_timeout(Duration::from_secs(1));
    let removed = remove_socket(socket, ours);
    served.map_err(|error| format!("board-server: {error}"))?;
    removed
}

/// Writes the start-up `line` to `out`; a closed or broken `out` never
/// stops the server (no panic, no error).
pub fn announce(out: &mut impl std::io::Write, line: &str) {
    let _ = out.write_all(line.as_bytes()).and_then(|()| out.flush());
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
