//! The document store behind every configuration layer (#2024): raw bytes
//! out, and for the repo-local overlay the one policy that decides whether
//! an entry at its location may be read as one.

use std::path::Path;

/// What the store found at an overlay candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverlayDocument {
    /// No directory entry at the overlay location.
    Absent,
    /// An entry exists but the overlay policy refuses to read it as the
    /// overlay; `reason` is what the report and every refusing command
    /// print. Nothing at the location is written through either.
    Refused { reason: String },
    /// The regular file's bytes.
    Present(Vec<u8>),
}

/// Reads configuration documents. Implemented by infrastructure over the
/// filesystem; faked in use-case tests.
pub trait ConfigDocumentStore: Send + Sync {
    /// The bytes at `path`, or `None` when no directory entry exists there.
    /// An entry that exists but cannot be read (a directory, a dangling
    /// symlink, a permission failure) is an error naming the reason — never
    /// `None`, so a present-but-broken file is reported rather than skipped.
    fn read(&self, path: &Path) -> Result<Option<Vec<u8>>, String>;

    /// The repo-local overlay at `path` (`<cwd>/.quecto/config.json`, see
    /// `OVERLAY_RELATIVE_PATH`), under the overlay policy: every directory
    /// entry below the working directory — the `.quecto` directory and the
    /// file — must be a regular one. A symbolic link anywhere on that
    /// stretch is [`OverlayDocument::Refused`] whatever it points at and
    /// whether or not a file lies behind it: trust is keyed by the file's
    /// identity, and a link would inherit the trust of its target — or hand
    /// a write to it. Otherwise as [`Self::read`]: absent, bytes, or an
    /// error naming a present-but-broken entry.
    fn read_overlay(&self, path: &Path) -> Result<OverlayDocument, String>;
}
