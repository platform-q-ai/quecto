//! Parent-side materialization of a proxy endpoint (#1369 slice 3).
//!
//! A create/exec result may carry `socket_proxy: {"argv": [...]}` instead of
//! a direct `socket_path`. The parent then owns a private bridge socket: each
//! connection accepted on it runs the validated proxy argv and pumps bytes
//! between the connection and the proxy process's stdio. Every existing
//! socket consumer — prompt routing, agent_cmd commands, and the
//! monitor's persistent liveness connection — connects to the bridge path the
//! launch captured, never to any requested direct path. When the child (or
//! its environment/proxy) dies, the proxy's stdout reaches EOF and the bridge
//! shuts the connection down, so death is pushed to the monitor as EOF with
//! no polling and no fake wrapper process.
//!
//! Every proxy process is owned by the one [`OwnedChildSupervisor`] (#1935).
//! The bound parent control connection (the monitor's) rides one bridged
//! connection like any other; its liveness is preserved end to end because
//! the proxy's stdin is the parent's pipe: when the parent dies — even by
//! SIGKILL — the pipe closes, the proxy exits (its contract is to exit on
//! stdin EOF; on Linux `PR_SET_PDEATHSIG` also ends it), and its connection
//! into the child closes, so the child sees its bound connection lost.
//! Nothing here ever reconnects a bridged connection on the parent's behalf.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::infrastructure::processes::owned_child_supervisor::{
    OwnedChildSupervisor, ProcessGroup, ProtocolOutcome, TerminationBudget,
};

/// One materialized proxy endpoint: the parent-side bridge socket path and
/// the accept-loop task serving it.
#[derive(Debug)]
pub(super) struct ProxyBridge {
    pub socket_path: PathBuf,
    handle: tokio::task::JoinHandle<()>,
}

impl ProxyBridge {
    /// Stop accepting new connections and remove the bridge socket file so
    /// nothing can connect to a dead child's bridge. In-flight bridged
    /// connections belong to their own tasks and end when either side closes.
    pub(super) fn teardown(&self) {
        self.handle.abort();
        let _ = std::fs::remove_file(&self.socket_path);
    }

    /// Hand the accept-loop handle and socket path to the registry entry that
    /// now owns their teardown.
    pub(super) fn into_parts(self) -> (PathBuf, tokio::task::JoinHandle<()>) {
        (self.socket_path, self.handle)
    }
}

/// Tear down a bridge owned by a registry entry: abort the accept loop and
/// remove the bridge socket file. Callable from any teardown path (terminal
/// EOF death, cascade removal, session shutdown).
pub(super) fn teardown_entry_bridge(
    handle: Option<&std::sync::Arc<tokio::task::JoinHandle<()>>>,
    socket_path: Option<&Path>,
) {
    if let Some(handle) = handle {
        handle.abort();
    }
    if let Some(path) = socket_path {
        let _ = std::fs::remove_file(path);
    }
}

/// Bind the parent-side bridge socket and start serving the proxy argv.
///
/// The bridge path is derived from the agent identity but deliberately
/// distinct from the requested direct child socket path: proxy mode never
/// touches (or falls back to) a requested direct path.
pub(super) fn materialize(
    argv: Vec<String>,
    socket_dir: &Path,
    agent_key: &str,
    supervisor: Arc<OwnedChildSupervisor>,
) -> std::io::Result<ProxyBridge> {
    let socket_path = socket_dir.join(format!("quecto-proxy-{agent_key}.sock"));
    let _ = std::fs::remove_file(&socket_path);
    let listener = tokio::net::UnixListener::bind(&socket_path)?;
    // Owner-only, same policy as every other quecto listening socket
    // (`bind_secure_socket`): the socket dir may be world-traversable (temp
    // fallback), and any local user who can connect could otherwise drive
    // the child agent through the bridge.
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))?;
    }
    let handle = tokio::spawn(accept_loop(listener, argv, supervisor));
    Ok(ProxyBridge {
        socket_path,
        handle,
    })
}

async fn accept_loop(
    listener: tokio::net::UnixListener,
    argv: Vec<String>,
    supervisor: Arc<OwnedChildSupervisor>,
) {
    loop {
        match listener.accept().await {
            Ok((conn, _)) => {
                tokio::spawn(bridge_one(conn, argv.clone(), Arc::clone(&supervisor)));
            }
            Err(e) => {
                tracing::warn!(error = %e, "proxy bridge: accept failed");
                return;
            }
        }
    }
}

/// Grace the proxy gets to exit on its own after its stdin is closed (its
/// protocol), before the supervisor's TERM/KILL fallback.
const PROXY_EXIT_BUDGET: TerminationBudget = TerminationBudget {
    exit_after_ack: std::time::Duration::from_millis(500),
    term_grace: std::time::Duration::from_millis(500),
    kill_grace: std::time::Duration::from_secs(2),
};

/// Serve one bridged connection: run the proxy argv and pump both directions,
/// racing them so either side closing tears the pair down.
///
/// - Proxy stdout closing (child or proxy died) shuts the parent connection
///   down so its reader observes EOF — death stays pushed.
/// - The parent connection closing (quecto clients never half-close their
///   write side, so read-side EOF means the connection is gone) ends the
///   proxy: its stdin is closed first, and the retained handle is only
///   signalled if it does not exit by itself. Without this, a dropped probe
///   or command connection would leak a live proxy process — and its open
///   connection into the child — for the child's entire lifetime.
async fn bridge_one(
    conn: tokio::net::UnixStream,
    argv: Vec<String>,
    supervisor: Arc<OwnedChildSupervisor>,
) {
    let mut cmd = tokio::process::Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::null());
    // Defence in depth: a parent killed outright takes its proxies with it
    // on Linux even if a proxy ignores stdin EOF.
    crate::infrastructure::processes::parent_death_signal::arm(&mut cmd, std::process::id());
    // Spawned and owned by the supervisor: the only holder of the process.
    let proxy = match supervisor.spawn(cmd, ProcessGroup::Inherited).await {
        Ok(proxy) => proxy,
        Err(e) => {
            tracing::warn!(error = %e, "proxy bridge: failed to spawn proxy argv");
            return;
        }
    };
    let handle = proxy.handle;
    let (Some(mut stdin), Some(mut stdout)) = (proxy.stdin, proxy.stdout) else {
        end_proxy(&supervisor, handle, None).await;
        return;
    };
    let (mut conn_read, mut conn_write) = conn.into_split();
    tokio::select! {
        _ = async {
            let _ = tokio::io::copy(&mut conn_read, &mut stdin).await;
        } => {
            // Parent connection closed: nobody is reading responses anymore.
        }
        _ = async {
            let _ = tokio::io::copy(&mut stdout, &mut conn_write).await;
        } => {
            // Proxy stdout closed (child death or proxy exit): push EOF to
            // the parent-side reader.
            use tokio::io::AsyncWriteExt;
            let _ = conn_write.shutdown().await;
        }
    }
    // The proxy's protocol is its stdin: closing it asks it to exit. Only
    // when it does not is the retained handle TERMed, then KILLed.
    end_proxy(&supervisor, handle, Some(stdin)).await;
}

async fn end_proxy(
    supervisor: &Arc<OwnedChildSupervisor>,
    handle: crate::infrastructure::processes::owned_child_supervisor::ChildHandleId,
    stdin: Option<tokio::process::ChildStdin>,
) {
    let protocol = async move {
        match stdin {
            Some(stdin) => {
                drop(stdin);
                ProtocolOutcome::Acknowledged
            }
            None => ProtocolOutcome::Negative("proxy stdio was not piped".into()),
        }
    };
    let outcome = supervisor
        .terminate(handle, protocol, PROXY_EXIT_BUDGET)
        .await;
    tracing::debug!(?outcome, "proxy bridge: proxy process ended");
    // One bridged connection, one proxy: nothing else observes its exit,
    // so the slot goes now, or as soon as a proxy that outlived its budget
    // is finally reaped.
    supervisor.retire_when_reaped(handle);
}

/// Wait until the child answers THROUGH the bridge (never via any direct
/// path): a probe connection whose proxy loses its child reads EOF and is
/// retried across the full readiness budget; ready only when a probe
/// survives its quiet window. Bounded retry, no lifecycle polling afterwards.
///
/// Residual assumption (documented in docs/container-runtimes.md): a proxy
/// that can neither reach the child nor fail with EOF — it simply hangs —
/// is indistinguishable from a live-but-quiet child until first real use;
/// the launch then fails at the initial prompt and rolls back.
pub(super) async fn wait_for_proxy_ready_until(
    socket_path: &Path,
    deadline: tokio::time::Instant,
) -> Result<(), crate::domain::error::DomainError> {
    use tokio::io::AsyncReadExt;
    loop {
        if let Ok(mut probe) = tokio::net::UnixStream::connect(socket_path).await {
            let mut byte = [0u8; 1];
            match tokio::time::timeout(std::time::Duration::from_millis(300), probe.read(&mut byte))
                .await
            {
                // No EOF within the probe window: the proxy holds a live
                // connection to the child. Ready.
                Err(_elapsed) => return Ok(()),
                // Child spoke or connection stayed open with data: ready.
                Ok(Ok(n)) if n > 0 => return Ok(()),
                // EOF/error: proxy could not reach the child yet.
                _ => {}
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(crate::domain::error::DomainError::Tool(format!(
                "subagent proxy endpoint {} did not become ready within 10s",
                socket_path.display()
            )));
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

#[cfg(test)]
#[path = "spawn_proxy_bridge_tests.rs"]
mod tests;
