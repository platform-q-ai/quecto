//! Pure session-home facts, separate from opaque identity and transcript layout.
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceGroup {
    Git { common_dir: PathBuf },
    Folder { directory: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssociationProvenance {
    SavedHere,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionHome {
    pub execution_dir: PathBuf,
    pub group: WorkspaceGroup,
    pub provenance: AssociationProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionHomeScope {
    Scoped(SessionHome),
    LegacyUnscoped,
    /// Present authority which cannot be interpreted is never legacy.
    Unavailable(String),
}

impl SessionHome {
    pub fn same_execution(&self, other: &Self) -> bool {
        self.execution_dir == other.execution_dir && self.group == other.group
    }
}

#[cfg(test)]
#[path = "session_home_tests.rs"]
mod tests;
