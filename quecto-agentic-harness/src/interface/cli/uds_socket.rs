//! UDS socket utilities: stale socket cleanup, secure binding, socket guard.

/// Result of probing a socket file for a live listener.
enum SocketLiveness {
    /// A listener accepted the probe connection.
    Live,
    /// The path exists but nothing accepts (dead agent or plain file).
    Dead,
    /// The probe was inconclusive (e.g. permission denied).
    Unknown,
}

/// Probe whether anything still accepts connections on `path`.
///
/// A connect that succeeds proves a live listener; `ECONNREFUSED` proves the
/// listener is gone (Linux also refuses connects to non-socket files, so a
/// stray plain file counts as dead). Anything else is inconclusive.
fn probe_socket_liveness(path: &std::path::Path) -> SocketLiveness {
    match std::os::unix::net::UnixStream::connect(path) {
        Ok(_) => SocketLiveness::Live,
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
            ) =>
        {
            SocketLiveness::Dead
        }
        Err(_) => SocketLiveness::Unknown,
    }
}

/// Remove dead `quecto-agent-*.sock` files from `dir`.
///
/// Liveness is decided by a connect probe, never by mtime: a socket file's
/// mtime is fixed at bind time, so any agent older than an age threshold
/// would look "stale" while still serving (#1460). A socket that accepts is
/// always kept; one that refuses is removed regardless of age. Only when the
/// probe is inconclusive does `max_age` apply as a conservative fallback.
pub fn reap_stale_sockets(dir: &std::path::Path, max_age: std::time::Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let cutoff = std::time::SystemTime::now()
        .checked_sub(max_age)
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
    for entry in entries.flatten() {
        let n = entry.file_name();
        let s = n.to_string_lossy();
        if !s.starts_with("quecto-agent-") || !s.ends_with(".sock") {
            continue;
        }
        match probe_socket_liveness(&entry.path()) {
            SocketLiveness::Live => {}
            SocketLiveness::Dead => {
                let _ = std::fs::remove_file(entry.path());
            }
            SocketLiveness::Unknown => {
                let is_stale = entry
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .is_some_and(|t| t < cutoff);
                if is_stale {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }
}

/// RAII guard that removes a socket file on drop.
pub(super) struct SocketGuard(pub std::path::PathBuf);

impl Drop for SocketGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Bind a Unix socket with mode 0600 (owner-only access).
pub(super) fn bind_secure_socket(
    path: &std::path::Path,
) -> std::io::Result<tokio::net::UnixListener> {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::remove_file(path);
    let listener = tokio::net::UnixListener::bind(path)?;
    if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
        let _ = std::fs::remove_file(path);
        return Err(e);
    }
    Ok(listener)
}
