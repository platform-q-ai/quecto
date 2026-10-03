//! The one physical layout of the flat session store (#1970).
//!
//! Every path a session's identity maps to on disk is formed here and
//! nowhere else: the session record, its ownership stamp, its retention
//! (spill) file and its image sidecars live in one flat `<base>/sessions/`
//! directory under the sanitized key. The file, lock and cache adapters own
//! only I/O; no caller outside this module joins `sessions`, sanitizes a key,
//! or forms a `.json`/`.owner`/`spill.jsonl` name. This is the seam a scoped
//! store changes later: its identity alters this projection, not the callers.
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

    /// `<base>/sessions/<sanitized key>.owner` — the single-writer lock's stamp (#1460).
    pub fn ownership_stamp(&self, identity: &SessionIdentity) -> PathBuf {
        self.sessions_dir
            .join(format!("{}.owner", self.sanitized(identity)))
    }

    /// `<base>/sessions/<sanitized key>/spill.jsonl` — the retention file (the
    /// ephemeral identity's is the sanitized empty key's: in-run retention, as before).
    pub fn spill_file(&self, identity: &SessionIdentity) -> PathBuf {
        self.sessions_dir
            .join(self.sanitized(identity))
            .join("spill.jsonl")
    }

    /// `<base>/sessions/<sanitized key>/images/` — the image sidecars (#2424).
    pub fn image_dir(&self, identity: &SessionIdentity) -> PathBuf {
        self.sessions_dir
            .join(self.sanitized(identity))
            .join("images")
    }

    /// Whether `file_name` (a directory entry) is a session record: the
    /// affirmative `.json` allowlist the list applies before reading.
    pub fn is_session_record(path: &Path) -> bool {
        path.extension().is_some_and(|ext| ext == "json")
    }

    /// The file-name prefix a record must start with to possibly belong to
    /// an identity under `prefix`: the sanitized prefix. Used only to skip
    /// files cheaply before their header is read (the parsed header's
    /// identity is the authority); a key that sanitizes to a hex name never
    /// passes it, the prefix optimisation's behaviour kept exactly.
    pub fn record_name_prefix(&self, prefix: &SessionKeyPrefix) -> String {
        super::filename::sanitize_session_key(prefix.as_str())
    }

    /// Optional authoritative home metadata, never rewritten by transcript saves.
    pub fn home_file(&self, identity: &SessionIdentity) -> PathBuf {
        self.sessions_dir
            .join(format!("{}.home", self.sanitized(identity)))
    }

    /// Discardable, atomically replaced home discovery index.
    pub fn home_catalogue_file(&self) -> PathBuf {
        self.sessions_dir.join("home.catalogue")
    }

    fn sanitized(&self, identity: &SessionIdentity) -> String {
        super::filename::sanitize_session_key(identity.runtime_key())
    }
}

#[cfg(test)]
#[path = "session_layout_tests.rs"]
mod tests;
