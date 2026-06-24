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
    /// edge case for this small local-file gate.
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
    #[cfg(any(test, feature = "test-support"))]
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

    /// Probe watched sources and return whether at least one content hash changed.
    pub fn sources_changed(&mut self) -> bool {
        let mut any_changed = false;
        for source in &mut self.sources {
            any_changed |= matches!(source.changed(), SourceChange::Changed);
        }
        any_changed
    }

    /// Record a successful reload.
    pub fn record_reloaded(&mut self, new: T) -> ReloadResult<T> {
        self.last_good = Some(new.clone());
        ReloadResult::Reloaded(new)
    }

    /// Poll watched sources and rebuild only when at least one content hash changed.
    #[cfg(any(test, feature = "test-support"))]
    pub fn poll(&mut self, rebuild: impl FnOnce() -> Result<T, String>) -> ReloadResult<T> {
        if !self.sources_changed() {
            return ReloadResult::Unchanged;
        }

        self.rebuild_or_keep_last_good(rebuild, "reload rebuild failed; keeping last-good")
    }

    /// Force a rebuild regardless of mtime/hash state.
    #[cfg(any(test, feature = "test-support"))]
    pub fn poll_forced(&mut self, rebuild: impl FnOnce() -> Result<T, String>) -> ReloadResult<T> {
        self.poll_forced_result(rebuild)
            .unwrap_or(ReloadResult::Unchanged)
    }

    /// Force a rebuild and preserve the rebuild error for command responses.
    #[cfg(any(test, feature = "test-support"))]
    pub fn poll_forced_result(
        &mut self,
        rebuild: impl FnOnce() -> Result<T, String>,
    ) -> Result<ReloadResult<T>, String> {
        match rebuild() {
            Ok(new) => Ok(self.record_reloaded(new)),
            Err(err) => {
                tracing::warn!(target: "reload", error = %err, "forced reload failed; keeping last-good");
                Err(err)
            }
        }
    }

    /// Last successfully rebuilt value.
    #[cfg(any(test, feature = "test-support"))]
    pub fn last_good(&self) -> Option<&T> {
        self.last_good.as_ref()
    }

    #[cfg(any(test, feature = "test-support"))]
    fn rebuild_or_keep_last_good(
        &mut self,
        rebuild: impl FnOnce() -> Result<T, String>,
        warning: &'static str,
    ) -> ReloadResult<T> {
        match rebuild() {
            Ok(new) => self.record_reloaded(new),
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
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn hash_changes_with_content() {
        assert_ne!(hash_bytes(b"a"), hash_bytes(b"b"));
    }

    #[test]
    fn runtime_reload_poll_rebuilds_on_changed_source() {
        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(b"initial").unwrap();
        tmp.flush().unwrap();

        let source = ReloadSource::new(tmp.path().to_path_buf());
        let mut gate = RuntimeReload::new(vec![source]);
        gate.seed(0usize);

        // First poll: unchanged (seed matches current content).
        let result = gate.poll(|| Ok(1usize));
        assert!(matches!(result, ReloadResult::Unchanged));

        // Change file.
        tmp.write_all(b"modified").unwrap();
        tmp.flush().unwrap();
        let result = gate.poll(|| Ok(2usize));
        assert!(matches!(result, ReloadResult::Reloaded(2)));
        assert_eq!(gate.last_good(), Some(&2));
    }

    #[test]
    fn reload_source_changed_detects_content_change() {
        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(b"initial").unwrap();
        tmp.flush().unwrap();

        let mut source = ReloadSource::new(tmp.path().to_path_buf());
        source.seed();
        assert!(matches!(
            source.changed(),
            SourceChange::UnchangedNoRead | SourceChange::Unchanged
        ));

        tmp.write_all(b"modified").unwrap();
        tmp.flush().unwrap();
        assert!(matches!(source.changed(), SourceChange::Changed));
    }

    #[test]
    fn reload_source_changed_returns_unchanged_for_same_mtime_length_and_hash() {
        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(b"initial").unwrap();
        tmp.flush().unwrap();

        let mut source = ReloadSource::new(tmp.path().to_path_buf());
        source.seed();
        // Reading again with same content returns unchanged without reading.
        assert!(matches!(source.changed(), SourceChange::UnchangedNoRead));
    }

    #[test]
    fn reload_source_changed_returns_missing_or_unreadable_for_missing_file() {
        let mut source = ReloadSource::new(PathBuf::from("/tmp/nonexistent-reload-12345"));
        assert!(matches!(
            source.changed(),
            SourceChange::MissingOrUnreadable
        ));
    }

    #[test]
    fn reload_source_changed_same_mtime_but_different_content() {
        use filetime::{FileTime, set_file_mtime};
        use std::io::Seek;

        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(b"alpha").unwrap();
        tmp.flush().unwrap();
        let mtime = FileTime::from_system_time(SystemTime::now());
        set_file_mtime(tmp.path(), mtime).unwrap();

        let mut source = ReloadSource::new(tmp.path().to_path_buf());
        source.seed();
        assert!(matches!(
            source.changed(),
            SourceChange::UnchangedNoRead | SourceChange::Unchanged
        ));

        // Same mtime, different length/content.
        tmp.rewind().unwrap();
        tmp.write_all(b"longer-beta").unwrap();
        tmp.flush().unwrap();
        set_file_mtime(tmp.path(), mtime).unwrap();
        assert!(matches!(source.changed(), SourceChange::Changed));
    }

    #[test]
    fn runtime_reload_poll_keeps_last_good_on_rebuild_error() {
        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(b"initial").unwrap();

        let source = ReloadSource::new(tmp.path().to_path_buf());
        let mut gate = RuntimeReload::new(vec![source]);
        gate.seed(0usize);

        tmp.write_all(b"modified").unwrap();
        let result = gate.poll(|| Err("boom".to_string()));
        assert!(matches!(result, ReloadResult::Unchanged));
        assert_eq!(gate.last_good(), Some(&0));
    }

    #[test]
    fn runtime_reload_poll_forced_rebuilds_even_when_unchanged() {
        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(b"initial").unwrap();

        let source = ReloadSource::new(tmp.path().to_path_buf());
        let mut gate = RuntimeReload::new(vec![source]);
        gate.seed(0usize);

        let result = gate.poll_forced(|| Ok(1usize));
        assert!(matches!(result, ReloadResult::Reloaded(1)));
    }

    #[test]
    fn runtime_reload_poll_forced_result_preserves_error() {
        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(b"initial").unwrap();

        let source = ReloadSource::new(tmp.path().to_path_buf());
        let mut gate = RuntimeReload::new(vec![source]);
        gate.seed(0usize);

        let result = gate.poll_forced_result(|| Err("boom".to_string()));
        assert!(result.is_err());
    }

    #[test]
    fn reload_source_last_mtime_matches_file() {
        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(b"x").unwrap();

        let mut source = ReloadSource::new(tmp.path().to_path_buf());
        source.seed();
        let mtime = source.last_mtime().unwrap();
        let meta = std::fs::metadata(tmp.path()).unwrap();
        assert_eq!(mtime, meta.modified().unwrap());
    }
}
