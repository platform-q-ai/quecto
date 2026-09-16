use super::*;
use std::process::Command;
fn git(path: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(path)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn folders_are_exact_and_missing_git_is_observable() {
    let dir = tempfile::tempdir().unwrap();
    let home = GitScopeDiscovery::default().discover(dir.path()).unwrap();
    assert_eq!(
        home.group,
        WorkspaceGroup::Folder {
            directory: dir.path().canonicalize().unwrap()
        }
    );
    assert!(
        GitScopeDiscovery {
            executable: dir.path().join("missing-git").into_os_string()
        }
        .discover(dir.path())
        .is_err()
    );
}
#[test]
fn nearest_repository_worktree_and_detached_head() {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
    git(
        dir.path(),
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.test",
            "commit",
            "--allow-empty",
            "-qm",
            "initial",
        ],
    );
    let discovery = GitScopeDiscovery::default();
    let root = discovery.discover(dir.path()).unwrap();
    let sub = dir.path().join("sub");
    std::fs::create_dir(&sub).unwrap();
    assert_eq!(root.group, discovery.discover(&sub).unwrap().group);
    let linked = dir.path().join("linked");
    git(
        dir.path(),
        &["worktree", "add", "--detach", linked.to_str().unwrap()],
    );
    let linked_home = discovery.discover(&linked).unwrap();
    assert_eq!(root.group, linked_home.group);
    assert_ne!(root.execution_dir, linked_home.execution_dir);
    git(&sub, &["init", "-q"]);
    assert_ne!(root.group, discovery.discover(&sub).unwrap().group);
}
#[test]
fn malformed_nested_marker_cannot_fall_back_to_parent() {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
    let nested = dir.path().join("nested");
    std::fs::create_dir_all(nested.join(".git")).unwrap();
    assert!(GitScopeDiscovery::default().discover(&nested).is_err());
}

#[test]
fn corrupt_repository_is_not_a_folder() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".git"), "gitdir: /missing/worktree").unwrap();
    assert!(GitScopeDiscovery::default().discover(dir.path()).is_err());
}
#[cfg(unix)]
#[test]
fn symlink_aliases_have_one_identity() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real");
    std::fs::create_dir(&real).unwrap();
    let alias = dir.path().join("alias");
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    let discovery = GitScopeDiscovery::default();
    assert_eq!(
        discovery.discover(&real).unwrap(),
        discovery.discover(&alias).unwrap()
    );
}

#[test]
fn worktree_membership_requires_one_nonbare_record() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let record = format!("worktree {}\0HEAD abc\0detached\0\0", root.display());
    assert!(validate_worktree_membership(record.as_bytes(), &root).is_ok());
    assert!(validate_worktree_membership(b"", &root).is_err());
    assert!(validate_worktree_membership(format!("{record}{record}").as_bytes(), &root).is_err());
    let bare = format!("worktree {}\0bare\0\0", root.display());
    assert!(validate_worktree_membership(bare.as_bytes(), &root).is_err());
}
