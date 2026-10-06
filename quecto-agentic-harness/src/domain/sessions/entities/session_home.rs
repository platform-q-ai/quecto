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

/// The one eligibility rule of a saved home, decided on pure facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeAdmission {
    /// The saved authority is re-observed unchanged at its directory, and that
    /// directory is the current execution directory in the same group.
    Eligible,
    /// The saved directory still exists but its group changed: a folder became
    /// a repository, or the repository's common dir moved.
    HomeChanged,
    /// The session was saved in another execution directory (a grouped
    /// worktree included): grouping never authorizes another directory.
    DifferentExecutionDirectory,
}

impl SessionHome {
    pub fn same_execution(&self, other: &Self) -> bool {
        self.execution_dir == other.execution_dir && self.group == other.group
    }

    /// Admit `saved` given `observed` (a fresh discovery at the saved
    /// execution directory) and `current` (a fresh discovery here). Every
    /// input is an affirmative observation; a missing one never admits.
    pub fn admission(saved: &Self, observed: &Self, current: &Self) -> HomeAdmission {
        if observed == saved && observed.same_execution(current) {
            HomeAdmission::Eligible
        } else if saved.execution_dir != current.execution_dir {
            HomeAdmission::DifferentExecutionDirectory
        } else {
            HomeAdmission::HomeChanged
        }
    }
}

#[cfg(test)]
#[path = "session_home_tests.rs"]
mod tests;
