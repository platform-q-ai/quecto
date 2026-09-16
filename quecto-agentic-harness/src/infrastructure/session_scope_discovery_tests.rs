use std::fs;

use crate::application::sessions::ports::scope_discovery::{ScopeDiscoveryOutcome, SessionScopeDiscovery};
use crate::domain::session_scope::SessionHomeScope;

use super::FilesystemGitScopeDiscovery;

fn temp(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("quecto-scope-{name}-{}", std::process::id()))
}

#[test]
fn canonicalizes_non_git_folder() {
    let root = temp("plain"); let child = root.join("child");
    let _ = fs::remove_dir_all(&root); fs::create_dir_all(&child).unwrap();
    let outcome = FilesystemGitScopeDiscovery.discover(&child);
    let ScopeDiscoveryOutcome::Discovered(SessionHomeScope::Scoped { execution_location, repository_grouping, .. }) = outcome else { panic!("expected discovered scope") };
    assert_eq!(execution_location.as_str(), child.canonicalize().unwrap().to_str().unwrap());
    assert_eq!(repository_grouping, None);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn nearest_nested_repository_wins() {
    let root = temp("nested"); let nested = root.join("outer/nested"); let child = nested.join("src");
    let _ = fs::remove_dir_all(&root); fs::create_dir_all(root.join("outer/.git")).unwrap(); fs::create_dir_all(nested.join(".git")).unwrap(); fs::create_dir_all(&child).unwrap();
    let outcome = FilesystemGitScopeDiscovery.discover(&child);
    let ScopeDiscoveryOutcome::Discovered(SessionHomeScope::Scoped { repository_grouping: Some(group), .. }) = outcome else { panic!("expected repository scope") };
    assert_eq!(group.worktree_git_dir(), nested.join(".git").canonicalize().unwrap().to_str().unwrap());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn linked_worktree_uses_gitdir_and_common_dir() {
    let root = temp("worktree"); let wt = root.join("wt"); let child = wt.join("src"); let gitdir = root.join("main/.git/worktrees/wt");
    let _ = fs::remove_dir_all(&root); fs::create_dir_all(&child).unwrap(); fs::create_dir_all(&gitdir).unwrap();
    fs::write(wt.join(".git"), "gitdir: ../main/.git/worktrees/wt\n").unwrap();
    fs::write(gitdir.join("commondir"), "../..\n").unwrap();
    let outcome = FilesystemGitScopeDiscovery.discover(&child);
    let ScopeDiscoveryOutcome::Discovered(SessionHomeScope::Scoped { repository_grouping: Some(group), .. }) = outcome else { panic!("expected worktree scope") };
    assert_eq!(group.common_git_dir(), root.join("main/.git").canonicalize().unwrap().to_str().unwrap());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn unavailable_and_ambiguous_inputs_are_explicit() {
    assert!(matches!(FilesystemGitScopeDiscovery.discover(&temp("missing")), ScopeDiscoveryOutcome::Unavailable { .. }));
    let root = temp("badgit"); let _ = fs::remove_dir_all(&root); fs::create_dir_all(&root).unwrap(); fs::write(root.join(".git"), "not a gitdir").unwrap();
    assert!(matches!(FilesystemGitScopeDiscovery.discover(&root), ScopeDiscoveryOutcome::Ambiguous { .. }));
    let _ = fs::remove_dir_all(root);
}
