//! Cross-process session-key ownership (#1460).
//!
//! A session key must have a single writer across processes: two agents
//! resuming the same key silently lose turns (`save_clean_delta` trusts the
//! caller's watermark, compaction races on a fixed `<key>.tmp`, and
//! `persisted_prefix_changed` resolves conflicts by discarding the local
//! delta). Ownership is an OS advisory lock (`flock(2)` via `File::try_lock`)
//! held on a stamp sidecar next to the session file for the guard's lifetime:
//!
//! - the kernel releases the lock on process death (including SIGKILL and
//!   OOM-kill), so a crash can never strand a key and pid recycling can never
//!   fake a live owner;
//! - acquisition is a single atomic `try_lock`, so two concurrent claimants
//!   can never both win (no read-remove-recreate reclaim window). It is retried
//!   briefly before refusing, because a lock belongs to the open file
//!   description: any concurrent `fork` in this process transiently duplicates
//!   the descriptor, so a released key can look held until the child `exec`s;
//! - the stamp file itself is never unlinked — unlinking would let a claimant
//!   lock an orphaned inode while a third process recreates the path, ending
//!   with two "owners". A left-over unlocked stamp is inert.
//!
//! The stamp's content is the owner's pid, written after the lock is won.
//! It is diagnostic only (used in refusal messages); the lock is the truth.

use std::path::{Path, PathBuf};

use crate::domain::error::DomainError;

/// Path of the ownership stamp sidecar for `key` under `sessions_dir`.
pub fn ownership_stamp_path(sessions_dir: &Path, key: &str) -> PathBuf {
    sessions_dir.join(format!(
        "{}.owner",
        super::filename::sanitize_session_key(key)
    ))
}

/// How long to keep retrying an exclusive claim before refusing.
///
/// A lock is held by the *open file description*, not the process. Spawning any
/// child forks the process, and between `fork` and `exec` the child holds a copy
/// of every descriptor — including one this process is about to close. Closing
/// it here therefore does not release the lock until the child reaches `exec`,
/// so a legitimate re-claim straight after a release can transiently see the key
/// as taken. Retrying rides out that window; a genuinely held key still refuses.
const LOCK_RETRY_BUDGET: std::time::Duration = std::time::Duration::from_millis(500);
const LOCK_RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_millis(10);

// std file locks stabilized in 1.89 (real toolchain floor; clippy.toml MSRV
// bump is pending).
#[expect(clippy::incompatible_msrv)]
fn try_lock_riding_out_forks(file: &std::fs::File) -> Result<(), std::fs::TryLockError> {
    let deadline = std::time::Instant::now() + LOCK_RETRY_BUDGET;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(()),
            Err(std::fs::TryLockError::WouldBlock) if std::time::Instant::now() < deadline => {
                std::thread::sleep(LOCK_RETRY_INTERVAL);
            }
            Err(e) => return Err(e),
        }
    }
}

/// The stamp records who won the lock. It is diagnostic only, and can name a
/// process that has since exited, so the wording never claims more than it knows.
fn describe_owner(owner: Option<u32>) -> String {
    match owner {
        Some(pid) if pid == std::process::id() => format!("this process ({pid})"),
        Some(pid) if pid_is_live(pid) => format!("live process {pid}"),
        Some(pid) => format!("process {pid}, which is no longer running"),
        None => "an unidentified process".to_string(),
    }
}

#[cfg(unix)]
fn pid_is_live(pid: u32) -> bool {
    // kill(0, ..) targets the caller's own process group rather than a process,
    // so it would report pid 0 as live. A stamp can only hold a real pid.
    if pid == 0 {
        return false;
    }
    // SAFETY: kill with signal 0 only performs existence/permission checking.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

#[cfg(not(unix))]
fn pid_is_live(_pid: u32) -> bool {
    true
}

/// Open (creating if needed, mode 0600) the stamp file for `key`.
///
/// Also used by tests to simulate a foreign owner: an independently opened
/// file description holding the exclusive lock behaves exactly as another
/// process would (`flock(2)` locks are per open file description).
pub fn open_stamp_file(sessions_dir: &Path, key: &str) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(ownership_stamp_path(sessions_dir, key))
}

/// RAII claim on a session key. Holding the guard marks the calling process
/// as the single writer for the key; dropping it (or process death) releases
/// the OS lock and with it the claim. The stamp file remains on disk but an
/// unlocked stamp carries no ownership.
#[derive(Debug)]
pub struct SessionOwnershipGuard {
    stamp_path: PathBuf,
    /// Holds the exclusive `flock` for the guard's lifetime.
    _lock_file: std::fs::File,
}

impl SessionOwnershipGuard {
    /// Claim ownership of `key` for the current process.
    ///
    /// Refused with an error naming the key and the stamped owner when
    /// another live process holds the lock. A stamp left behind by a dead
    /// process carries no lock and is reclaimed transparently.
    //
    // `File::try_lock` stabilized in 1.89; the crate's tests already call the
    // std file-lock APIs, so 1.89 is the real toolchain floor — clippy.toml's
    // declared 1.85 predates the #1460 locking work and awaits a coordinated
    // MSRV bump.
    pub fn acquire(sessions_dir: &Path, key: &str) -> Result<Self, DomainError> {
        std::fs::create_dir_all(sessions_dir).map_err(|e| {
            DomainError::Session(format!("failed to create sessions dir for ownership: {e}"))
        })?;
        let stamp_path = ownership_stamp_path(sessions_dir, key);
        let file = open_stamp_file(sessions_dir, key).map_err(|e| {
            DomainError::Session(format!("failed to open ownership stamp for '{key}': {e}"))
        })?;
        match try_lock_riding_out_forks(&file) {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => {
                // Diagnostic only: the owner wrote its pid after locking.
                let owner = std::fs::read_to_string(&stamp_path)
                    .ok()
                    .and_then(|s| s.trim().parse::<u32>().ok());
                let owner = describe_owner(owner);
                return Err(DomainError::Session(format!(
                    "session key '{key}' is owned by {owner} \
                     (stamp {}): refusing a second writer — close the \
                     owning agent or pick another session",
                    stamp_path.display()
                )));
            }
            Err(std::fs::TryLockError::Error(e)) => {
                return Err(DomainError::Session(format!(
                    "failed to claim session key '{key}': {e}"
                )));
            }
        }
        // Lock won: record our pid for refusal diagnostics.
        use std::io::Write;
        let mut writer = &file;
        let stamp_write = file
            .set_len(0)
            .and_then(|()| writer.write_all(std::process::id().to_string().as_bytes()));
        if let Err(e) = stamp_write {
            return Err(DomainError::Session(format!(
                "failed to write ownership stamp: {e}"
            )));
        }
        Ok(Self {
            stamp_path,
            _lock_file: file,
        })
    }

    /// The stamp file backing this claim.
    pub fn stamp_path(&self) -> &Path {
        &self.stamp_path
    }
}

/// Per-store registry of claimed keys: a key is claimed on first use and
/// held until the registry (its store) is dropped, so two processes can
/// never silently interleave writes to one session key (#1460).
#[derive(Debug, Default)]
pub struct SessionOwnershipRegistry {
    owned: std::sync::Mutex<std::collections::HashMap<String, SessionOwnershipGuard>>,
}

impl SessionOwnershipRegistry {
    /// Ensure the calling process owns `key`, claiming it on first use.
    /// A key locked by another live process is refused with an explicit
    /// error instead of silently losing turns.
    pub fn claim(&self, sessions_dir: &Path, key: &str) -> Result<(), DomainError> {
        let mut owned = self.owned.lock().expect("session ownership mutex");
        if owned.contains_key(key) {
            return Ok(());
        }
        let guard = SessionOwnershipGuard::acquire(sessions_dir, key)?;
        owned.insert(key.to_string(), guard);
        Ok(())
    }

    pub fn release(&self, key: &str) {
        let mut owned = self.owned.lock().expect("session ownership mutex");
        owned.remove(key);
    }
}

#[cfg(test)]
#[path = "session_ownership_tests.rs"]
mod tests;
