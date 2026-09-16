//! Scope discovery ports for folder-aware sessions (#2001, D2).
//!
//! Application-facing contracts only: path canonicalization and workspace
//! discovery. Adapters perform filesystem and Git work; these ports name
//! domain values and affirmative eligibility outcomes (never denylists).

use crate::domain::error::DomainError;
use crate::domain::session_home_scope::{
    CanonicalExecutionLocation, RepositoryLabel, RepositoryWorktreeGrouping, SessionHomeScope,
};

/// Port: turn a caller path into a [`CanonicalExecutionLocation`].
///
/// Infrastructure resolves symlinks and absolute form; domain never does.
pub trait PathCanonicalizer: Send + Sync {
    fn canonicalize(&self, path: &str) -> Result<CanonicalExecutionLocation, DomainError>;
}

/// Affirmative workspace discovery outcomes for one invocation path.
///
/// Every variant is an allowlisted result the application can branch on;
/// adapters never invent silent fallbacks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceDiscoveryOutcome {
    /// Git nearest/nested repository resolved; nested wins over parent.
    GitRepository {
        scope: SessionHomeScope,
        /// Repository common/root path as Git reported it (canonical text).
        repository_root: CanonicalExecutionLocation,
        /// True when discovery walked up from a subdirectory into the repo root.
        invoked_from_subdirectory: bool,
    },
    /// No Git repository: exact canonical folder identity.
    NonGitExactFolder { scope: SessionHomeScope },
    /// Git is installed but the outcome is ambiguous (e.g. conflicting facts).
    GitAmbiguous { reason: String },
    /// Git is unavailable (missing binary, permission, or probe failure).
    GitUnavailable { reason: String },
}

impl WorkspaceDiscoveryOutcome {
    /// Affirmative: discovery produced a usable home scope for local listing.
    pub fn has_usable_scope(&self) -> bool {
        matches!(
            self,
            Self::GitRepository { .. } | Self::NonGitExactFolder { .. }
        )
    }

    pub fn scope(&self) -> Option<&SessionHomeScope> {
        match self {
            Self::GitRepository { scope, .. } | Self::NonGitExactFolder { scope } => Some(scope),
            Self::GitAmbiguous { .. } | Self::GitUnavailable { .. } => None,
        }
    }

    pub fn is_git_repository(&self) -> bool {
        matches!(self, Self::GitRepository { .. })
    }

    pub fn is_non_git_exact_folder(&self) -> bool {
        matches!(self, Self::NonGitExactFolder { .. })
    }

    pub fn is_git_ambiguous(&self) -> bool {
        matches!(self, Self::GitAmbiguous { .. })
    }

    pub fn is_git_unavailable(&self) -> bool {
        matches!(self, Self::GitUnavailable { .. })
    }
}

/// Port: discover the local session home scope for a working directory.
pub trait WorkspaceDiscovery: Send + Sync {
    /// Discover scope for `invocation_path` (typically the process CWD text
    /// before or after canonicalization — adapters decide the sequence).
    fn discover(&self, invocation_path: &str) -> Result<WorkspaceDiscoveryOutcome, DomainError>;
}

/// Helper builders used by adapters and tests when assembling domain scopes.
pub fn scoped_git_home(
    execution: CanonicalExecutionLocation,
    repository_label: RepositoryLabel,
    member_locations: Vec<CanonicalExecutionLocation>,
) -> SessionHomeScope {
    let grouping = RepositoryWorktreeGrouping::related_worktrees(
        repository_label,
        execution.clone(),
        member_locations,
    );
    SessionHomeScope::scoped(execution, Some(grouping))
}

pub fn scoped_exact_folder(execution: CanonicalExecutionLocation) -> SessionHomeScope {
    SessionHomeScope::scoped(execution, None)
}

#[cfg(test)]
#[path = "workspace_discovery_tests.rs"]
mod tests;
