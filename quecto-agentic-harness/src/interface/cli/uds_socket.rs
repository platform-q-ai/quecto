//! UDS socket utilities: stale socket cleanup, secure binding, socket guard.

/// Result of probing a socket file for a live listener.
enum SocketLiveness {
    /// A live endpoint is bound to the path.
    Live,
    /// The path exists but no live endpoint is bound to it.
    Dead,
    /// The probe was inconclusive (e.g. the kernel table is unreadable).
    Unknown,
}

/// Probe whether a live endpoint is still bound to `path`.
///
/// Reads the kernel's unix-socket table (`/proc/net/unix`) instead of
/// connecting (#1468 review): a connect probe is indistinguishable from a
/// real client attach, so its immediate disconnect can trip the
/// last-client-gone shutdown of a live non-persist agent, and `connect(2)`
/// can block indefinitely against a full accept backlog. The table lists
/// every bound filesystem-path socket in this network namespace; an entry
/// vanishing proves the endpoint is gone. (A live agent bound in another
/// network namespace would not appear here and could be misjudged dead —
/// agents and reaper share the host namespace today.)
fn probe_socket_liveness(path: &std::path::Path) -> SocketLiveness {
    let Ok(table) = std::fs::read("/proc/net/unix") else {
        return SocketLiveness::Unknown;
    };
    let table = String::from_utf8_lossy(&table);
    let target = path.to_string_lossy();
    let live = table.lines().skip(1).any(|line| {
        // The path is the trailing field; require the preceding space so a
        // path that is a suffix of another cannot false-positive.
        line.strip_suffix(target.as_ref())
            .is_some_and(|rest| rest.ends_with(' '))
    });
    if live {
        SocketLiveness::Live
    } else {
        SocketLiveness::Dead
    }
}

/// Remove dead `quecto-agent-*.sock` files from `dir`.
///
/// Liveness is decided by a kernel socket-table probe, never by mtime: a
/// socket file's mtime is fixed at bind time, so any agent older than an age
/// threshold would look "stale" while still serving (#1460). A path with a
/// live bound endpoint is always kept; one without is removed regardless of
/// age. Only when the probe is inconclusive does `max_age` apply as a
/// conservative fallback.
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
