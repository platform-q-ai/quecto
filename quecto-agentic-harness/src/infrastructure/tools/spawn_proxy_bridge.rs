//! Parent-side materialization of a proxy endpoint (#1369 slice 3).
//!
//! A create/exec result may carry `socket_proxy: {"argv": [...]}` instead of
//! a direct `socket_path`. The parent then owns a private bridge socket: each
//! connection accepted on it runs the validated proxy argv and pumps bytes
//! between the connection and the proxy process's stdio. Every existing
//! socket consumer — prompt routing, agent_cmd commands, await, and the
//! monitor's persistent liveness connection — connects to the bridge path the
//! launch captured, never to any requested direct path. When the child (or
//! its environment/proxy) dies, the proxy's stdout reaches EOF and the bridge
//! shuts the connection down, so death is pushed to the monitor as EOF with
//! no polling and no fake wrapper process.

use std::path::{Path, PathBuf};

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
    let handle = tokio::spawn(accept_loop(listener, argv));
    Ok(ProxyBridge {
        socket_path,
        handle,
    })
}

async fn accept_loop(listener: tokio::net::UnixListener, argv: Vec<String>) {
    loop {
        match listener.accept().await {
            Ok((conn, _)) => {
                tokio::spawn(bridge_one(conn, argv.clone()));
            }
            Err(e) => {
                tracing::warn!(error = %e, "proxy bridge: accept failed");
                return;
            }
        }
    }
}

/// Serve one bridged connection: run the proxy argv and pump both directions,
/// racing them so either side closing tears the pair down.
///
/// - Proxy stdout closing (child or proxy died) shuts the parent connection
///   down so its reader observes EOF — death stays pushed.
/// - The parent connection closing (quecto clients never half-close their
///   write side, so read-side EOF means the connection is gone) kills the
///   proxy immediately. Without this, a dropped probe or await connection
///   would leak a live proxy process — and its open connection into the
///   child — for the child's entire lifetime.
async fn bridge_one(conn: tokio::net::UnixStream, argv: Vec<String>) {
    let mut cmd = tokio::process::Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::null());
    cmd.kill_on_drop(true);
    let mut proxy = match cmd.spawn() {
        Ok(proxy) => proxy,
        Err(e) => {
            tracing::warn!(error = %e, "proxy bridge: failed to spawn proxy argv");
            return;
        }
    };
    let Some(mut stdin) = proxy.stdin.take() else {
        return;
    };
    let Some(mut stdout) = proxy.stdout.take() else {
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
    let _ = proxy.kill().await;
    let _ = proxy.wait().await;
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
pub(super) async fn wait_for_proxy_ready(
    socket_path: &Path,
) -> Result<(), crate::domain::error::DomainError> {
    use tokio::io::AsyncReadExt;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
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
