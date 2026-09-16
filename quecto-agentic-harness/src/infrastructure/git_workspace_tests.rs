//! Git workspace probe integration tests (#2001 D2).

use super::*;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn git(cwd: &std::path::Path, args: &[&str]) {
    let st = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .expect("git");
    assert!(st.success(), "git {args:?} failed");
}

fn init_repo(dir: &std::path::Path) {
    fs::create_dir_all(dir).unwrap();
    git(dir, &["init"]);
    git(dir, &["config", "user.email", "t@example.com"]);
    git(dir, &["config", "user.name", "t"]);
    fs::write(dir.join("README"), "x").unwrap();
    git(dir, &["add", "README"]);
    git(dir, &["commit", "-m", "init"]);
}

#[test]
fn nearest_repository_from_subdirectory() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("myrepo");
    init_repo(&repo);
    let sub = repo.join("src").join("deep");
    fs::create_dir_all(&sub).unwrap();
    let probe = GitCliProbe::new().probe(&sub);
    match probe {
        GitProbe::Inside {
            toplevel,
            worktrees,
            label,
            ..
        } => {
            let expected = fs::canonicalize(&repo).unwrap();
            assert_eq!(toplevel.as_str(), expected.to_str().unwrap());
            assert!(worktrees.iter().any(|w| w.as_str() == toplevel.as_str()));
            assert_eq!(label.as_str(), "myrepo");
        }
        other => panic!("expected Inside, got {other:?}"),
    }
}

#[test]
fn nested_repository_wins_over_parent() {
    let tmp = TempDir::new().unwrap();
    let parent = tmp.path().join("parent");
    init_repo(&parent);
    let nested = parent.join("vendor").join("nested");
    init_repo(&nested);
    let probe = GitCliProbe::new().probe(&nested);
    match probe {
        GitProbe::Inside { toplevel, label, .. } => {
            let expected = fs::canonicalize(&nested).unwrap();
            assert_eq!(toplevel.as_str(), expected.to_str().unwrap());
            assert_eq!(label.as_str(), "nested");
        }
        other => panic!("expected nested Inside, got {other:?}"),
    }
}

#[test]
fn non_git_directory_is_not_a_repository() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("plain");
    fs::create_dir_all(&dir).unwrap();
    assert_eq!(GitCliProbe::new().probe(&dir), GitProbe::NotARepository);
}

#[test]
fn related_worktrees_are_listed_when_present() {
    let tmp = TempDir::new().unwrap();
    let main = tmp.path().join("main");
    init_repo(&main);
    let wt = tmp.path().join("feature-wt");
    git(&main, &["branch", "feature"]);
    git(
        &main,
        &[
            "worktree",
            "add",
            wt.to_str().unwrap(),
            "feature",
        ],
    );
    let probe = GitCliProbe::new().probe(&main);
    match probe {
        GitProbe::Inside { worktrees, .. } => {
            assert!(
                worktrees.len() >= 2,
                "expected main + feature worktree, got {worktrees:?}"
            );
            let main_c = fs::canonicalize(&main).unwrap();
            let wt_c = fs::canonicalize(&wt).unwrap();
            assert!(worktrees.iter().any(|w| w.as_str() == main_c.to_str().unwrap()));
            assert!(worktrees.iter().any(|w| w.as_str() == wt_c.to_str().unwrap()));
        }
        other => panic!("expected Inside with worktrees, got {other:?}"),
    }
}
