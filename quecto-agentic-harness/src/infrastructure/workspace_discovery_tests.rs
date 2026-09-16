//! Workspace discovery integration tests (#2001 D2).

use super::*;
use crate::application::sessions::ports::WorkspaceDiscovery;
use crate::domain::session_home_scope::SessionHomeScope;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn git(cwd: &std::path::Path, args: &[&str]) {
    let st = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .status()
        .expect("git");
    assert!(st.success(), "git {args:?}");
}

fn init_repo(dir: &std::path::Path) {
    fs::create_dir_all(dir).unwrap();
    git(dir, &["init"]);
    fs::write(dir.join("README"), "x").unwrap();
    git(dir, &["add", "README"]);
    git(dir, &["-c", "user.email=t@example.com", "-c", "user.name=t", "commit", "-m", "init"]);
}

#[test]
fn discovers_git_scope_from_repo_subdirectory() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("app");
    init_repo(&repo);
    let sub = repo.join("lib");
    fs::create_dir_all(&sub).unwrap();
    let d = DefaultWorkspaceDiscovery::new();
    let outcome = d.discover(sub.to_str().unwrap()).unwrap();
    assert!(outcome.has_usable_scope());
    assert!(outcome.is_git_repository());
    match outcome {
        WorkspaceDiscoveryOutcome::GitRepository {
            scope,
            repository_root,
            invoked_from_subdirectory,
        } => {
            assert!(invoked_from_subdirectory);
            let expected_root = fs::canonicalize(&repo).unwrap();
            assert_eq!(repository_root.as_str(), expected_root.to_str().unwrap());
            match scope {
                SessionHomeScope::Scoped {
                    execution_location,
                    repository_grouping,
                } => {
                    let expected_exec = fs::canonicalize(&sub).unwrap();
                    assert_eq!(execution_location.as_str(), expected_exec.to_str().unwrap());
                    let g = repository_grouping.expect("git grouping");
                    assert_eq!(g.repository_label().as_str(), "app");
                    assert!(g.contains_location(&repository_root));
                }
                SessionHomeScope::LegacyUnscoped => panic!("expected scoped"),
            }
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn discovers_exact_folder_outside_git() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("notes");
    fs::create_dir_all(&dir).unwrap();
    let d = DefaultWorkspaceDiscovery::new();
    let outcome = d.discover(dir.to_str().unwrap()).unwrap();
    assert!(outcome.is_non_git_exact_folder());
    let scope = outcome.scope().unwrap();
    assert!(scope.repository_grouping().is_none());
    let expected = fs::canonicalize(&dir).unwrap();
    assert_eq!(
        scope.execution_location().unwrap().as_str(),
        expected.to_str().unwrap()
    );
}

#[test]
fn nested_git_repo_wins() {
    let tmp = TempDir::new().unwrap();
    let parent = tmp.path().join("parent");
    init_repo(&parent);
    let nested = parent.join("inner");
    init_repo(&nested);
    let d = DefaultWorkspaceDiscovery::new();
    let outcome = d.discover(nested.to_str().unwrap()).unwrap();
    match outcome {
        WorkspaceDiscoveryOutcome::GitRepository {
            repository_root, ..
        } => {
            let expected = fs::canonicalize(&nested).unwrap();
            assert_eq!(repository_root.as_str(), expected.to_str().unwrap());
        }
        other => panic!("expected git nested, {other:?}"),
    }
}

#[test]
fn symlink_invocation_uses_canonical_target() {
    use std::os::unix::fs::symlink;
    let tmp = TempDir::new().unwrap();
    let real = tmp.path().join("real");
    init_repo(&real);
    let link = tmp.path().join("via-link");
    symlink(&real, &link).unwrap();
    let d = DefaultWorkspaceDiscovery::new();
    let outcome = d.discover(link.to_str().unwrap()).unwrap();
    match outcome {
        WorkspaceDiscoveryOutcome::GitRepository {
            scope,
            repository_root,
            ..
        } => {
            let expected = fs::canonicalize(&real).unwrap();
            assert_eq!(repository_root.as_str(), expected.to_str().unwrap());
            assert_eq!(
                scope.execution_location().unwrap().as_str(),
                expected.to_str().unwrap()
            );
        }
        other => panic!("expected git via symlink, {other:?}"),
    }
}
