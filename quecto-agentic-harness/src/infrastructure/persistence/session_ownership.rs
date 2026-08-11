//! Cross-process session-key ownership (#1460).
//!
//! A session key must have a single writer across processes: two agents
//! resuming the same key silently lose turns (`save_clean_delta` trusts the
//! caller's watermark, compaction races on a fixed `<key>.tmp`, and
//! `persisted_prefix_changed` resolves conflicts by discarding the local
//! delta). Ownership is claimed via a pid stamp sidecar next to the session
//! file; a claim on a key whose stamped owner is still alive is refused with
//! an explicit error, while a stamp left by a dead process (or an unreadable
//! stamp) is reclaimed so a crash can never strand a key.

use std::path::{Path, PathBuf};

use crate::domain::error::DomainError;

/// Path of the ownership stamp sidecar for `key` under `sessions_dir`.
pub fn ownership_stamp_path(sessions_dir: &Path, key: &str) -> PathBuf {
    sessions_dir.join(format!(
        "{}.owner",
        super::filename::sanitize_session_key(key)
    ))
}

/// Whether a process with `pid` is currently alive.
///
/// `kill(pid, 0)` delivers no signal but performs the existence check;
/// `EPERM` still proves the pid exists (it belongs to another user).
fn pid_is_alive(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    // SAFETY: signal 0 sends nothing; this is the canonical existence probe.
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// RAII claim on a session key. Holding the guard marks the calling process
/// as the single writer for the key; dropping it releases the claim.
#[derive(Debug)]
pub struct SessionOwnershipGuard {
    stamp_path: PathBuf,
}

impl SessionOwnershipGuard {
    /// Claim ownership of `key` for the current process.
    pub fn acquire(sessions_dir: &Path, key: &str) -> Result<Self, DomainError> {
        Self::acquire_as(sessions_dir, key, std::process::id())
    }

    /// Claim ownership of `key` on behalf of `owner_pid` (test seam: lets
    /// tests simulate claims from other live/dead processes).
    pub fn acquire_as(sessions_dir: &Path, key: &str, owner_pid: u32) -> Result<Self, DomainError> {
        std::fs::create_dir_all(sessions_dir).map_err(|e| {
            DomainError::Session(format!("failed to create sessions dir for ownership: {e}"))
        })?;
        let stamp_path = ownership_stamp_path(sessions_dir, key);
        // One reclaim attempt: if the existing stamp names a dead process (or
        // is unreadable), remove it and retry the exclusive create once.
        for attempt in 0..2 {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&stamp_path)
            {
                Ok(file) => {
                    use std::io::Write;
                    let mut file = file;
                    file.write_all(owner_pid.to_string().as_bytes())
                        .map_err(|e| {
                            DomainError::Session(format!("failed to write ownership stamp: {e}"))
                        })?;
                    return Ok(Self { stamp_path });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && attempt == 0 => {
                    let stamped_owner = std::fs::read_to_string(&stamp_path)
                        .ok()
                        .and_then(|s| s.trim().parse::<u32>().ok());
                    if let Some(pid) = stamped_owner
                        && pid_is_alive(pid)
                    {
                        return Err(DomainError::Session(format!(
                            "session key '{key}' is owned by live process {pid} \
                             (stamp {}): refusing a second writer — close the \
                             owning agent or pick another session",
                            stamp_path.display()
                        )));
                    }
                    // Dead owner or unreadable stamp: reclaim.
                    let _ = std::fs::remove_file(&stamp_path);
                }
                Err(e) => {
                    return Err(DomainError::Session(format!(
                        "failed to claim session key '{key}': {e}"
                    )));
                }
            }
        }
        unreachable!("second create_new attempt always returns")
    }

    /// The stamp file backing this claim.
    pub fn stamp_path(&self) -> &Path {
        &self.stamp_path
    }
}

/// Per-store registry of claimed keys: a key is claimed on first write and
/// held until the registry (its store) is dropped, so two processes can
/// never silently interleave writes to one session key (#1460).
#[derive(Debug, Default)]
pub struct SessionOwnershipRegistry {
    owned: std::sync::Mutex<std::collections::HashMap<String, SessionOwnershipGuard>>,
}

impl SessionOwnershipRegistry {
    /// Ensure the calling process owns `key`, claiming it on first use.
    /// A key whose stamped owner is another live process is refused with an
    /// explicit error instead of silently losing turns.
    pub fn claim(&self, sessions_dir: &Path, key: &str) -> Result<(), DomainError> {
        let mut owned = self.owned.lock().expect("session ownership mutex");
        if owned.contains_key(key) {
            return Ok(());
        }
        let guard = SessionOwnershipGuard::acquire(sessions_dir, key)?;
        owned.insert(key.to_string(), guard);
        Ok(())
    }
}

impl Drop for SessionOwnershipGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.stamp_path);
    }
}

#[cfg(test)]
#[path = "session_ownership_tests.rs"]
mod tests;
