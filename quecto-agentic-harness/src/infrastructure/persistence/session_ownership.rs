//! Cross-process session-key ownership (#1460).
//!
//! A session key must have a single writer across processes: two agents
//! resuming the same key silently lose turns (`save_clean_delta` trusts the
//! caller's watermark, compaction races on a fixed `<key>.tmp`, and
//! `persisted_prefix_changed` resolves conflicts by discarding the local
//! delta). Ownership is claimed via a pid stamp sidecar next to the session
//! file; a claim on a key whose stamped owner is still alive is refused with
//! an explicit error, while a stamp left by a dead process is reclaimed.
//!
//! RED stub (#1460): this currently mirrors today's behavior — no ownership
//! is recorded or enforced. The tests in `session_ownership_tests.rs` pin the
//! target contract and fail against this stub.

use std::path::{Path, PathBuf};

use crate::domain::error::DomainError;

/// Path of the ownership stamp sidecar for `key` under `sessions_dir`.
pub fn ownership_stamp_path(sessions_dir: &Path, key: &str) -> PathBuf {
    sessions_dir.join(format!(
        "{}.owner",
        super::filename::sanitize_session_key(key)
    ))
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
        // RED stub: no stamp written, no liveness check, never refuses.
        let _ = owner_pid;
        Ok(Self {
            stamp_path: ownership_stamp_path(sessions_dir, key),
        })
    }

    /// The stamp file backing this claim.
    pub fn stamp_path(&self) -> &Path {
        &self.stamp_path
    }
}

#[cfg(test)]
#[path = "session_ownership_tests.rs"]
mod tests;
