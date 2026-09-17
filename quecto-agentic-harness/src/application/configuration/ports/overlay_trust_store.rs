//! Trust of the repo-local overlay (#2024): a checked-out project's
//! `.quecto/config.json` is applied only once its exact content has been
//! approved, identified by canonical path and content fingerprint.

use std::path::{Path, PathBuf};

/// The trust decision for one overlay content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverlayTrust {
    /// This exact content at this path has been approved.
    Trusted,
    /// Not approved; `fingerprint` identifies the content so the user can
    /// approve it explicitly.
    Untrusted { fingerprint: String },
}

/// The receipt of an explicit approval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayApproval {
    /// The path the approval was recorded under (canonical when resolvable).
    pub path: PathBuf,
    /// The approved content's fingerprint.
    pub fingerprint: String,
}

/// Decides and records overlay trust. Implemented by infrastructure over a
/// per-user trust record (an adapter may also ask an interactive user);
/// faked in use-case tests.
pub trait OverlayTrustStore: Send + Sync {
    /// Whether this exact content at this path is recorded as approved.
    fn decide(&self, path: &Path, content: &[u8]) -> OverlayTrust;

    /// Ask the user, when there is one to ask, whether to trust this
    /// content — shown to them, so what they consent to is what the caller
    /// then [`approve`](Self::approve)s; `true` is consent, not a record.
    /// A non-interactive adapter answers `false`.
    fn offer(&self, path: &Path, fingerprint: &str, content: &[u8]) -> bool;

    /// Record this exact content at this path as the one trusted content
    /// for the path (an earlier approval of other content is superseded).
    fn approve(&self, path: &Path, content: &[u8]) -> Result<OverlayApproval, String>;
}
