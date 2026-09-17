//! The one write path of configuration documents (#2024).

use std::path::Path;

/// An exclusive hold on one configuration document, released on drop.
/// Whatever the adapter locks with (a sidecar `flock`), holding it means
/// no other patch of the same document — in this process or another — is
/// between its read and its write.
pub trait DocumentLock: Send {}

/// The no-lock lock, for adapters (and fakes) that need none.
impl DocumentLock for () {}

/// Replaces (or creates) a configuration document so that a reader sees
/// either the previous content or the whole new content, never a prefix,
/// and so that a document already in the writer's layout changes only on
/// the lines whose values changed (key order and unknown keys preserved;
/// the existing indentation kept). Implemented by infrastructure
/// (tmp + fsync + rename); faked in use-case tests.
pub trait ConfigDocumentWriter: Send + Sync {
    /// Write `document` and return the exact bytes laid down, so a caller
    /// that records trust records what it wrote, not what is on disk later.
    fn write(&self, path: &Path, document: &serde_json::Value) -> Result<Vec<u8>, String>;

    /// Take the exclusive hold on `path` a read → patch → validate → write
    /// cycle runs under, so concurrent patches of one document serialise
    /// and none loses another's update. Blocks until it is free.
    fn exclusive(&self, path: &Path) -> Result<Box<dyn DocumentLock>, String>;
}
