//! What a record's metadata says, read in one place: why a retained record
//! is kept (#1924, #2681) and where its members' git checkout is (#2682).
use std::path::PathBuf;

use super::{EnvironmentRecord, EnvironmentStatus};

/// The metadata key naming why a retained record is kept (#1924).
pub(super) const RETAINED_KEY: &str = "retained";

/// The create-result metadata key naming the members' checkout (#1924).
pub const CHECKOUT_KEY: &str = "checkout";

impl EnvironmentRecord {
    /// Why a `retained` record is kept (#2681): the reason
    /// [`Self::retain_with`] recorded, the one owner of that text. `None`
    /// for any other status, or a retained record that names no reason.
    pub fn retained_reason(&self) -> Option<&str> {
        match self.status {
            EnvironmentStatus::Retained => self
                .metadata
                .get(RETAINED_KEY)
                .and_then(serde_json::Value::as_str)
                .filter(|reason| !reason.is_empty()),
            EnvironmentStatus::Running
            | EnvironmentStatus::Killing
            | EnvironmentStatus::Stopped
            | EnvironmentStatus::CleanupFailed => None,
        }
    }

    /// The git checkout the members work in (#2682): the create result's
    /// absolute `metadata.checkout` (the shipped scripts clone into
    /// `<workspace>/repo`), else the workspace itself. What a parent
    /// fetches a kept box's commits from.
    pub fn checkout(&self) -> PathBuf {
        match self.metadata.get(CHECKOUT_KEY) {
            Some(serde_json::Value::String(path)) if std::path::Path::new(path).is_absolute() => {
                PathBuf::from(path)
            }
            _ => self.workspace_path.clone(),
        }
    }
}

#[cfg(test)]
#[path = "record_metadata_tests.rs"]
mod tests;
