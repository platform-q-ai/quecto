//! Unit tests for workspace discovery port vocabulary (#2001 D2).
//! Pure port types — no filesystem or Git.

use super::*;
use crate::domain::session_home_scope::{
    CanonicalExecutionLocation, RepositoryLabel, SessionHomeScope,
};

#[test]
fn outcome_git_repository_exposes_usable_scope() {
    let execution = CanonicalExecutionLocation::from_canonical_path("/work/repo/src");
    let root = CanonicalExecutionLocation::from_canonical_path("/work/repo");
    let scope = scoped_git_home(
        execution.clone(),
        RepositoryLabel::new("repo").unwrap(),
        vec![root.clone()],
    );
    let outcome = WorkspaceDiscoveryOutcome::GitRepository {
        scope: scope.clone(),
        repository_root: root,
        invoked_from_subdirectory: true,
    };
    assert!(outcome.has_usable_scope());
    assert!(outcome.is_git_repository());
    assert!(!outcome.is_non_git_exact_folder());
    assert_eq!(outcome.scope(), Some(&scope));
    assert!(outcome.scope().unwrap().is_scoped());
}

#[test]
fn outcome_non_git_exact_folder_is_usable_without_grouping() {
    let execution = CanonicalExecutionLocation::from_canonical_path("/tmp/notes");
    let scope = scoped_exact_folder(execution);
    let outcome = WorkspaceDiscoveryOutcome::NonGitExactFolder {
        scope: scope.clone(),
    };
    assert!(outcome.has_usable_scope());
    assert!(outcome.is_non_git_exact_folder());
    assert!(!outcome.is_git_repository());
    assert!(outcome.scope().unwrap().repository_grouping().is_none());
}

#[test]
fn ambiguous_and_unavailable_are_explicit_non_usable_outcomes() {
    let ambiguous = WorkspaceDiscoveryOutcome::GitAmbiguous {
        reason: "conflicting worktree list".into(),
    };
    let unavailable = WorkspaceDiscoveryOutcome::GitUnavailable {
        reason: "git binary missing".into(),
    };
    assert!(ambiguous.is_git_ambiguous());
    assert!(unavailable.is_git_unavailable());
    assert!(!ambiguous.has_usable_scope());
    assert!(!unavailable.has_usable_scope());
    assert!(ambiguous.scope().is_none());
    assert!(unavailable.scope().is_none());
}

#[test]
fn helpers_build_domain_scopes_without_path_derivation() {
    let loc = CanonicalExecutionLocation::from_canonical_path("/a");
    let git = scoped_git_home(
        loc.clone(),
        RepositoryLabel::new("a").unwrap(),
        vec![loc.clone()],
    );
    match git {
        SessionHomeScope::Scoped {
            execution_location,
            repository_grouping,
        } => {
            assert_eq!(execution_location.as_str(), "/a");
            assert!(repository_grouping.is_some());
        }
        SessionHomeScope::LegacyUnscoped => panic!("expected scoped"),
    }
    let exact = scoped_exact_folder(loc);
    assert!(exact.repository_grouping().is_none());
}
