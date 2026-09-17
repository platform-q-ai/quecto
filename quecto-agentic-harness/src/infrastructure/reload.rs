//! Runtime reload gate for startup-loaded file-backed surfaces.
//!
//! This module owns only the shared pull-based change-detection mechanism from
//! ADR-0002: stat, hash, seed, fail safe. What a change rebuilds is the
//! reload use case's, driven through the source adapter in
//! `runtime_configuration.rs` (#1849).

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Result of probing a watched file-backed source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceChange {
    /// The cached mtime and length matched the filesystem metadata, so the file was not read.
    UnchangedNoRead,
    /// The mtime moved, but the content hash was unchanged.
    Unchanged,
    /// The content hash changed and the caller should rebuild live state.
    Changed,
    /// The source could not be statted or read. Keep last-good state.
    MissingOrUnreadable,
}

/// A file-backed source whose content may change at runtime.
#[derive(Debug, Clone)]
pub struct ReloadSource {
    path: PathBuf,
    last_mtime: Option<SystemTime>,
    last_len: Option<u64>,
    last_hash: u64,
    removal_is_change: bool,
    /// A path that must exist for this source to count at all (#2024):
    /// the overlay for its trust record. While the guard is absent the
    /// source is neither fingerprinted nor reported; when the guard
    /// (re)appears the source is unseeded, so the rebuild that the
    /// guard's own change prompts seeds it — or, failing that, the next
    /// probe reports it.
    guard: Option<PathBuf>,
}

impl ReloadSource {
    /// Create an unseeded reload source. A file that goes missing after it
    /// was seen is fail-safe (`MissingOrUnreadable`, last-good state kept):
    /// an editor's save window or a deleted base config must not rebuild a
    /// live session against defaults.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            last_mtime: None,
            last_len: None,
            last_hash: 0,
            removal_is_change: false,
            guard: None,
        }
    }

    /// Watch this source only while `guard` exists: a record shared by
    /// every repository on the host is of interest to a session only
    /// while that session's overlay is there to be trusted.
    pub fn while_present(self, guard: impl Into<PathBuf>) -> Self {
        Self {
            guard: Some(guard.into()),
            ..self
        }
    }

    /// A source whose removal *is* a change (#2024): the repo-local
    /// overlay and its trust record, which are optional by design, so
    /// deleting one must take effect on the next reload.
    pub fn optional(path: impl Into<PathBuf>) -> Self {
        Self {
            removal_is_change: true,
            ..Self::new(path)
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Seed the source fingerprint from disk without reporting a change.
    /// A guarded source whose guard is absent is left (or put back)
    /// unseeded.
    pub fn seed(&mut self) {
        if !self.guard_present() {
            self.forget();
            return;
        }
        let Ok((mtime, len, hash)) = read_fingerprint(&self.path) else {
            return;
        };
        self.last_mtime = Some(mtime);
        self.last_len = Some(len);
        self.last_hash = hash;
    }

    /// Probe the source and report whether its content changed.
    ///
    /// The cheap no-read path requires both mtime and file length to match the
    /// last observation. Length is included so same-mtime rewrites that add or
    /// remove provider config bytes are still read and detected even on coarse
    /// timestamp filesystems. Same-mtime same-length rewrites remain a tolerated
    /// edge case for this small local-file gate.
    ///
    /// The observed fingerprint advances on every successful read. In
    /// particular, a touch-only update advances the mtime cache so subsequent
    /// polls are stat-only no-ops.
    pub fn changed(&mut self) -> SourceChange {
        if !self.guard_present() {
            self.forget();
            return SourceChange::UnchangedNoRead;
        }
        let Ok(metadata) = fs::metadata(&self.path) else {
            return self.vanished();
        };
        let Ok(mtime) = metadata.modified() else {
            return self.vanished();
        };

        let len = metadata.len();
        if self.last_mtime == Some(mtime) && self.last_len == Some(len) {
            return SourceChange::UnchangedNoRead;
        }

        let Ok(bytes) = fs::read(&self.path) else {
            return SourceChange::MissingOrUnreadable;
        };
        let hash = hash_bytes(&bytes);

        self.last_mtime = Some(mtime);
        self.last_len = Some(len);
        if self.last_hash == hash {
            SourceChange::Unchanged
        } else {
            self.last_hash = hash;
            SourceChange::Changed
        }
    }

    /// An optional source that was present at the last observation and is
    /// missing now is a change; a required one, or one never seen, keeps
    /// the last-good state.
    fn vanished(&mut self) -> SourceChange {
        if self.removal_is_change && self.last_mtime.take().is_some() {
            self.last_len = None;
            self.last_hash = 0;
            SourceChange::Changed
        } else {
            SourceChange::MissingOrUnreadable
        }
    }

    fn guard_present(&self) -> bool {
        self.guard.as_ref().is_none_or(|guard| guard.exists())
    }

    /// Drop the fingerprint: the next observation starts from nothing.
    fn forget(&mut self) {
        self.last_mtime = None;
        self.last_len = None;
        self.last_hash = 0;
    }

    /// Last observed mtime, exposed for state-machine tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn last_mtime(&self) -> Option<SystemTime> {
        self.last_mtime
    }
}

/// The change-detection gate over every file-backed source one runtime was
/// composed from: seeded once at startup, probed at the natural
/// checkpoints (ADR-0002). What to rebuild on a change, and the last-good
/// state a failed rebuild retains, are the reload use case's and the
/// runtime's — the gate holds only fingerprints.
#[derive(Debug, Clone)]
pub struct RuntimeReload {
    sources: Vec<ReloadSource>,
}

impl RuntimeReload {
    /// Create an unseeded reload gate.
    pub fn new(sources: Vec<ReloadSource>) -> Self {
        Self { sources }
    }

    /// Observe every source's current fingerprint without reporting a
    /// change: at startup, and again before a rebuild reads the files so a
    /// later probe reports only edits made after that read.
    pub fn seed(&mut self) {
        for source in &mut self.sources {
            source.seed();
        }
    }

    /// Probe watched sources and return whether at least one content hash
    /// changed. Every probe advances the observed fingerprints, so a file
    /// that is not edited again reports unchanged from then on — even if
    /// the rebuild this probe prompted fails.
    pub fn sources_changed(&mut self) -> bool {
        let mut any_changed = false;
        for source in &mut self.sources {
            any_changed |= matches!(source.changed(), SourceChange::Changed);
        }
        any_changed
    }
}

fn read_fingerprint(path: &PathBuf) -> Result<(SystemTime, u64, u64), std::io::Error> {
    let metadata = fs::metadata(path)?;
    let mtime = metadata.modified()?;
    let len = metadata.len();
    let bytes = fs::read(path)?;
    Ok((mtime, len, hash_bytes(&bytes)))
}

fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
#[path = "reload_tests.rs"]
mod tests;
