use super::*;
use std::process::Command;
fn git(path: &Path, args: &[&str]) {
    let output = Command::new("git")
        // A hook-run test inherits GIT_DIR/GIT_WORK_TREE; they must not redirect the fixture.
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
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

#[test]
fn not_a_repository_is_recognised_at_root_and_at_filesystem_boundaries() {
    use std::os::unix::process::ExitStatusExt;
    let failed = std::process::ExitStatus::from_raw(128 << 8);
    assert!(is_not_a_repository(
        &failed,
        b"fatal: not a git repository (or any of the parent directories): .git\n"
    ));
    assert!(is_not_a_repository(
        &failed,
        b"fatal: not a git repository (or any parent up to mount point /)\nStopping at filesystem boundary (GIT_DISCOVERY_ACROSS_FILESYSTEM not set).\n"
    ));
    assert!(!is_not_a_repository(
        &failed,
        b"fatal: detected dubious ownership in repository at '/srv/repo'\n"
    ));
    assert!(!is_not_a_repository(
        &std::process::ExitStatus::from_raw(1 << 8),
        b"fatal: not a git repository (or any of the parent directories): .git\n"
    ));
}

/// A writable directory whose device differs from its parent's: a mount
/// boundary Git's parent walk stops at. `None` when this machine has none.
fn mount_boundary_directory() -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let mut candidates = vec![
        std::env::temp_dir(),
        PathBuf::from("/tmp"),
        "/dev/shm".into(),
    ];
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        candidates.push(runtime.into());
    }
    candidates.into_iter().find(|dir| {
        let parent_dev = dir
            .parent()
            .and_then(|parent| std::fs::metadata(parent).ok())
            .map(|m| m.dev());
        std::fs::metadata(dir).ok().map(|m| m.dev()) != parent_dev
            && parent_dev.is_some()
            && tempfile::tempdir_in(dir).is_ok()
    })
}

#[test]
fn non_git_folder_on_a_mount_boundary_is_an_exact_folder() {
    let Some(boundary) = mount_boundary_directory() else {
        eprintln!("no writable mount boundary on this machine; classifier test covers the message");
        return;
    };
    let dir = tempfile::tempdir_in(&boundary).unwrap();
    let home = GitScopeDiscovery::default().discover(dir.path()).unwrap();
    assert_eq!(
        home.group,
        WorkspaceGroup::Folder {
            directory: dir.path().canonicalize().unwrap()
        }
    );
}

/// L4: the marker walk stops where Git's parent walk stops — at the
/// filesystem boundary — so a `.git` above a mount point is never consulted.
#[test]
fn marker_walk_stops_at_the_filesystem_boundary_git_stopped_at() {
    let Some(boundary) = mount_boundary_directory() else {
        eprintln!("no writable mount boundary on this machine; walk covers the same device");
        return;
    };
    let dir = tempfile::tempdir_in(&boundary).unwrap();
    let nested = dir.path().join("a").join("b");
    std::fs::create_dir_all(&nested).unwrap();
    let canonical = nested.canonicalize().unwrap();
    let boundary = boundary.canonicalize().unwrap();
    let walk = marker_walk(&canonical).unwrap();
    assert_eq!(walk.first().unwrap(), &canonical);
    assert_eq!(
        walk.last().unwrap(),
        &boundary,
        "the mount point itself is examined, nothing above it: {walk:?}"
    );
    assert!(
        !walk.iter().any(|dir| dir == boundary.parent().unwrap()),
        "{walk:?}"
    );
    assert!(verify_no_git_marker(&canonical).is_ok());
}

#[test]
fn marker_walk_on_one_filesystem_reaches_every_ancestor() {
    let dir = tempfile::tempdir().unwrap();
    let canonical = dir.path().canonicalize().unwrap();
    let walk = marker_walk(&canonical).unwrap();
    let same_device: Vec<_> = canonical
        .ancestors()
        .take_while(|ancestor| {
            use std::os::unix::fs::MetadataExt;
            std::fs::metadata(ancestor).map(|m| m.dev()).ok()
                == std::fs::metadata(&canonical).map(|m| m.dev()).ok()
        })
        .map(Path::to_path_buf)
        .collect();
    assert_eq!(walk, same_device);
    assert!(marker_walk(&canonical.join("missing")).is_err());
}

#[test]
fn git_is_resolved_to_an_absolute_program_once() {
    let discovery = GitScopeDiscovery::default();
    assert!(
        Path::new(&discovery.executable).is_absolute(),
        "{:?} must be resolved on PATH so std uses posix_spawn",
        discovery.executable
    );
    assert!(resolve_on_path("quecto-no-such-program-2009").is_none());
}

#[tokio::test]
async fn async_discovery_matches_blocking_discovery() {
    let dir = tempfile::tempdir().unwrap();
    let discovery = GitScopeDiscovery::default();
    assert_eq!(
        discovery.discover_async(dir.path()).await.unwrap(),
        discovery.discover(dir.path()).unwrap()
    );
}
