//! UDS socket utilities: stale socket cleanup, secure binding, socket guard.

/// Remove stale quecto-agent-*.sock files older than `max_age`.
pub(crate) fn reap_stale_sockets(dir: &std::path::Path, max_age: std::time::Duration) {
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
