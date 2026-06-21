//! Runtime reload gate for startup-loaded file-backed surfaces.
//!
//! This module owns only the shared pull-based change-detection mechanism from
//! ADR-0002: stat, hash, seed, fail safe. Surface-specific rebuilds are supplied
//! by callers.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
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
}

impl ReloadSource {
    /// Create an unseeded reload source.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            last_mtime: None,
            last_len: None,
            last_hash: 0,
        }
    }

    /// Seed the source fingerprint from disk without reporting a change.
    pub fn seed(&mut self) {
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
    /// edge case for this small local-file gate; callers that need a stronger
    /// guarantee can use `poll_forced`.
    ///
    /// The observed fingerprint advances on every successful read. In
    /// particular, a touch-only update advances the mtime cache so subsequent
    /// polls are stat-only no-ops.
    pub fn changed(&mut self) -> SourceChange {
        let Ok(metadata) = fs::metadata(&self.path) else {
            return SourceChange::MissingOrUnreadable;
        };
        let Ok(mtime) = metadata.modified() else {
            return SourceChange::MissingOrUnreadable;
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

    /// Last observed mtime, exposed for state-machine tests.
    pub fn last_mtime(&self) -> Option<SystemTime> {
        self.last_mtime
    }
}

/// Result of polling a runtime reload gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReloadResult<T> {
    Unchanged,
    Reloaded(T),
}

/// Shared reload gate plus fail-safe last-good state.
#[derive(Debug, Clone)]
pub struct RuntimeReload<T> {
    sources: Vec<ReloadSource>,
    last_good: Option<T>,
}

impl<T: Clone> RuntimeReload<T> {
    /// Create an unseeded reload gate.
    pub fn new(sources: Vec<ReloadSource>) -> Self {
        Self {
            sources,
            last_good: None,
        }
    }

    /// Seed all source fingerprints and store the initial last-good value.
    pub fn seed(&mut self, initial: T) {
        for source in &mut self.sources {
            source.seed();
        }
        self.last_good = Some(initial);
    }

    /// Poll watched sources and rebuild only when at least one content hash changed.
    pub fn poll(&mut self, rebuild: impl FnOnce() -> Result<T, String>) -> ReloadResult<T> {
        let mut any_changed = false;
        for source in &mut self.sources {
            any_changed |= matches!(source.changed(), SourceChange::Changed);
        }

        if !any_changed {
            return ReloadResult::Unchanged;
        }

        self.rebuild_or_keep_last_good(rebuild, "reload rebuild failed; keeping last-good")
    }

    /// Force a rebuild regardless of mtime/hash state.
    pub fn poll_forced(&mut self, rebuild: impl FnOnce() -> Result<T, String>) -> ReloadResult<T> {
        self.rebuild_or_keep_last_good(rebuild, "forced reload failed; keeping last-good")
    }

    /// Last successfully rebuilt value.
    pub fn last_good(&self) -> Option<&T> {
        self.last_good.as_ref()
    }

    fn rebuild_or_keep_last_good(
        &mut self,
        rebuild: impl FnOnce() -> Result<T, String>,
        warning: &'static str,
    ) -> ReloadResult<T> {
        match rebuild() {
            Ok(new) => {
                for source in &mut self.sources {
                    source.seed();
                }
                self.last_good = Some(new.clone());
                ReloadResult::Reloaded(new)
            }
            Err(err) => {
                tracing::warn!(target: "reload", error = %err, warning);
                ReloadResult::Unchanged
            }
        }
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
mod tests {
    use super::*;

    #[test]
    fn hash_changes_with_content() {
        assert_ne!(hash_bytes(b"a"), hash_bytes(b"b"));
    }
}
