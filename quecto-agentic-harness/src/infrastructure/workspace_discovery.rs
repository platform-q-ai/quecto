//! Workspace discovery adapter composing path canonicalization and Git probe (#2001 D2).

use std::path::Path;

use crate::application::sessions::ports::{
    scoped_exact_folder, scoped_git_home, PathCanonicalizer, WorkspaceDiscovery,
    WorkspaceDiscoveryOutcome,
};
use crate::domain::error::DomainError;
use crate::domain::session_home_scope::CanonicalExecutionLocation;
use crate::infrastructure::git_workspace::{GitCliProbe, GitProbe};
use crate::infrastructure::path_canonicalizer::FsPathCanonicalizer;

/// Default discovery: canonicalize invocation path, then Git-nearest or exact folder.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultWorkspaceDiscovery {
    canonicalizer: FsPathCanonicalizer,
    git: GitCliProbe,
}

impl DefaultWorkspaceDiscovery {
    pub fn new() -> Self {
        Self {
            canonicalizer: FsPathCanonicalizer::new(),
            git: GitCliProbe::new(),
        }
    }

    /// Test/composition seam with injected ports.
    pub fn with_parts(canonicalizer: FsPathCanonicalizer, git: GitCliProbe) -> Self {
        Self {
            canonicalizer,
            git,
        }
    }
}

impl WorkspaceDiscovery for DefaultWorkspaceDiscovery {
    fn discover(&self, invocation_path: &str) -> Result<WorkspaceDiscoveryOutcome, DomainError> {
        let execution = self.canonicalizer.canonicalize(invocation_path)?;
        let path = Path::new(execution.as_str());
        match self.git.probe(path) {
            GitProbe::Inside {
                toplevel,
                worktrees,
                label,
                ..
            } => {
                let invoked_from_subdirectory = execution.as_str() != toplevel.as_str();
                // Session execution directory is the invocation location (distinguished).
                let scope = scoped_git_home(execution.clone(), label, worktrees);
                Ok(WorkspaceDiscoveryOutcome::GitRepository {
                    scope,
                    repository_root: toplevel,
                    invoked_from_subdirectory,
                })
            }
            GitProbe::NotARepository => {
                let scope = scoped_exact_folder(execution);
                Ok(WorkspaceDiscoveryOutcome::NonGitExactFolder { scope })
            }
            GitProbe::Ambiguous { reason } => {
                Ok(WorkspaceDiscoveryOutcome::GitAmbiguous { reason })
            }
            GitProbe::Unavailable { reason } => {
                Ok(WorkspaceDiscoveryOutcome::GitUnavailable { reason })
            }
        }
    }
}

/// Affirmative: only usable outcomes yield a scope for local session listing.
pub fn local_scope_from_outcome(
    outcome: &WorkspaceDiscoveryOutcome,
) -> Option<&crate::domain::session_home_scope::SessionHomeScope> {
    outcome.scope()
}

#[allow(dead_code)] // composition will use; keep type name stable for D2
fn _assert_execution_is_location(loc: &CanonicalExecutionLocation) -> &str {
    loc.as_str()
}

#[cfg(test)]
#[path = "workspace_discovery_tests.rs"]
mod tests;
