//! The one physical layout of the flat session store (#1970).
//!
//! Every path a session's identity maps to on disk is formed here and
//! nowhere else: the session record, its ownership stamp and its retention
//! (spill) file all live in one flat `<base>/sessions/` directory under the
//! sanitized key. The file, lock and cache adapters (`FileSessionStore`,
//! `SessionOwnershipRegistry`, `FileContextSpillStore`) consume the returned
//! paths and own only I/O mechanics; no caller outside this module joins
//! `sessions`, sanitizes a key, or forms a `.json`/`.owner`/`spill.jsonl`
//! name.
//!
//! This is the seam a folder/workspace-scoped store changes later: a scoped
//! identity alters this projection (and the identity), not the callers.
use std::path::{Path, PathBuf};

use crate::domain::session_identity::{SessionIdentity, SessionKeyPrefix};

/// The flat `<base>/sessions/<sanitized key>` projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlatSessionLayout {
    sessions_dir: PathBuf,
}

impl FlatSessionLayout {
    /// The layout under `<base_dir>/sessions`.
    pub fn new(base_dir: impl AsRef<Path>) -> Self {
        Self {
            sessions_dir: base_dir.as_ref().join("sessions"),
        }
    }

    /// The directory every session record and stamp lives in (the list
    /// walks it; the writers create it).
    pub fn sessions_dir(&self) -> &Path {
        &self.sessions_dir
    }

    /// `<base>/sessions/<sanitized key>.json` — the session record.
    pub fn session_file(&self, identity: &SessionIdentity) -> PathBuf {
        self.sessions_dir
            .join(format!("{}.json", self.sanitized(identity)))
    }

    /// `<base>/sessions/<sanitized key>.owner` — the single-writer stamp
    /// the ownership lock is held on (#1460).
    pub fn ownership_stamp(&self, identity: &SessionIdentity) -> PathBuf {
        self.sessions_dir
            .join(format!("{}.owner", self.sanitized(identity)))
    }

    /// `<base>/sessions/<sanitized key>/spill.jsonl` — the retention file.
    /// The ephemeral identity projects to the sanitized empty key, so an
    /// ephemeral run's in-run retention has a file exactly as before.
    pub fn spill_file(&self, identity: &SessionIdentity) -> PathBuf {
        self.sessions_dir
            .join(self.sanitized(identity))
            .join("spill.jsonl")
    }

    /// Whether `file_name` (a directory entry) is a session record: the
    /// affirmative `.json` allowlist the list applies before reading.
    pub fn is_session_record(path: &Path) -> bool {
        path.extension().is_some_and(|ext| ext == "json")
    }

    /// The file-name prefix a record must start with to possibly belong to
    /// an identity under `prefix`: the sanitized prefix. Used only to skip
    /// files cheaply before their header is read; the identity check on the
    /// parsed header remains the authority. A key that sanitizes to a hex
    /// name never passes this filter — the existing behaviour of the
    /// prefix optimisation, kept exactly.
    pub fn record_name_prefix(&self, prefix: &SessionKeyPrefix) -> String {
        super::filename::sanitize_session_key(prefix.as_str())
    }

    fn sanitized(&self, identity: &SessionIdentity) -> String {
        super::filename::sanitize_session_key(identity.runtime_key())
    }
}

#[cfg(test)]
#[path = "session_layout_tests.rs"]
mod tests;
