//! The document store behind every configuration layer (#2024): raw bytes
//! out, presence in.

use std::path::Path;

/// Reads configuration documents. Implemented by infrastructure over the
/// filesystem; faked in use-case tests.
pub trait ConfigDocumentStore: Send + Sync {
    /// The bytes at `path`, or `None` when no directory entry exists there.
    /// An entry that exists but cannot be read (a directory, a dangling
    /// symlink, a permission failure) is an error naming the reason — never
    /// `None`, so a present-but-broken file is reported rather than skipped.
    fn read(&self, path: &Path) -> Result<Option<Vec<u8>>, String>;

    /// Whether any directory entry exists at `path`.
    fn is_present(&self, path: &Path) -> bool;
}
