//! The one filesystem fact configuration selection depends on: whether a
//! candidate local file is present, and if so whether it is usable.

use std::path::Path;

/// What the adapter found at a candidate local configuration path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalConfigPresence {
    /// No directory entry at all — the only outcome that permits fallback.
    Absent,
    /// A readable regular file (a symlink to one counts).
    RegularFile,
    /// An entry that is not a regular file (directory, socket, …).
    NotRegularFile,
    /// An entry that exists but whose target cannot be inspected (a dangling
    /// symlink, a symlink loop, a parent directory that cannot be searched),
    /// with the reason. A regular file whose *contents* cannot be read is
    /// reported as [`RegularFile`](Self::RegularFile); the loader reports
    /// that failure with the path.
    Unreadable(String),
}

/// Probes a candidate local configuration path. Implemented by
/// infrastructure over the filesystem; faked in use-case tests.
pub trait LocalConfigProbe: Send + Sync {
    fn probe(&self, path: &Path) -> LocalConfigPresence;
}
