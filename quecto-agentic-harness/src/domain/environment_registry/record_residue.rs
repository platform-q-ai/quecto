//! What a record says about what its box left behind (#2134): where its
//! state directory lives, and whether it is a plain `stopped` record or an
//! older build's relabel of a retained one (round 4 L3, #2033).
use std::path::PathBuf;

use super::{EnvironmentRecord, EnvironmentStatus};

/// The last error a record gone at restore carries. A display string and,
/// with a `retained` reason, the older build's relabel signature
/// [`EnvironmentRecord::relabelled_while_retained`] matches: hence here.
pub const GONE_AT_RESTORE: &str = "container not found at restore: the runtime reports it gone";

impl EnvironmentRecord {
    /// The environment's state directory: the workspace ancestor named
    /// after the environment id (`<root>/<environment_id>/workspace[/repo]`),
    /// or `None` when the workspace names no such directory.
    pub fn environment_dir(&self) -> Option<PathBuf> {
        self.workspace_path
            .ancestors()
            .find(|ancestor| {
                ancestor
                    .file_name()
                    .is_some_and(|name| name == self.environment_id.as_str())
            })
            .map(PathBuf::from)
    }

    /// An older build's restore relabelled this record `stopped` while it
    /// was retained: its own retention reason under a `stopped` status with
    /// the restore's last error alone is that build's signature.
    pub fn relabelled_while_retained(&self) -> bool {
        self.status == EnvironmentStatus::Stopped
            && self.metadata.get("retained").is_some_and(|v| v.is_string())
            && self.last_error.as_deref() == Some(GONE_AT_RESTORE)
    }

    /// Stopped by a completed kill (or found gone at a restore): terminal,
    /// its box torn down. An older build's relabel is retained in truth.
    pub fn is_plain_stopped(&self) -> bool {
        self.status == EnvironmentStatus::Stopped && !self.relabelled_while_retained()
    }
}

#[cfg(test)]
#[path = "record_residue_tests.rs"]
mod tests;
