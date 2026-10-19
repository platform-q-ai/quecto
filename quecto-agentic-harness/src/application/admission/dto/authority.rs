//! What the authority administration reports: a quota group's counts, the
//! authority's state, a reset's outcome, and why an admin operation failed.

use std::collections::BTreeMap;
use std::path::PathBuf;

/// A quota group's operator-facing counts, as the authority reports them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityGroupReport {
    pub active: usize,
    pub queued: usize,
    pub uncertain: usize,
    pub cooldown_until_ms: u64,
    pub unavailable: bool,
}

/// The authority's state as `status` reads it, tagged with the directory it
/// was read from so every output names the broker it addressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityReport {
    pub directory: PathBuf,
    pub epoch: u64,
    pub journal_healthy: bool,
    pub live_scopes: usize,
    pub groups: BTreeMap<String, AuthorityGroupReport>,
}

/// The outcome of a reset: the new epoch and the directory addressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetReport {
    pub directory: PathBuf,
    pub epoch: u64,
}

/// Why an admin operation could not complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorityAdminError {
    /// No broker is reachable at the addressed directory.
    NotRunning { directory: PathBuf, reason: String },
    /// The broker answered but the operation failed.
    Failed { directory: PathBuf, reason: String },
}

impl std::fmt::Display for AuthorityAdminError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotRunning { directory, reason } => write!(
                f,
                "no admission broker is running for directory {} ({reason})",
                directory.display()
            ),
            Self::Failed { directory, reason } => {
                write!(
                    f,
                    "admission broker at {} failed: {reason}",
                    directory.display()
                )
            }
        }
    }
}
