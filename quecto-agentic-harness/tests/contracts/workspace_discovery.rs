#[test]
fn real_workspace_port_groups_repository_subdirectories_but_not_execution() {
    use quecto::application::sessions::ports::session_home::WorkspaceDiscovery;
    use quecto::infrastructure::workspace::git_scope_discovery::GitScopeDiscovery;
    let dir = tempfile::tempdir().unwrap();
    assert!(
        std::process::Command::new("git")
            // A hook-run test inherits GIT_DIR/GIT_WORK_TREE; they must not redirect `init`.
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .arg("init")
            .arg(dir.path())
            .output()
            .unwrap()
            .status
            .success()
    );
    let sub = dir.path().join("sub");
    std::fs::create_dir(&sub).unwrap();
    let discovery: &dyn WorkspaceDiscovery = &GitScopeDiscovery::default();
    let root = discovery.discover(dir.path()).unwrap();
    let child = discovery.discover(&sub).unwrap();
    assert_eq!(root.group, child.group);
    assert!(!root.same_execution(&child));
}

/// R2-M2: the production adapter is immune to ambient `GIT_DIR`,
/// `GIT_WORK_TREE` and `GIT_CEILING_DIRECTORIES` — a hook, an IDE or a
/// wrapper exporting them must not redefine the requested directory's scope.
/// Deleting the adapter's `env_clear()` fails this test.
#[test]
fn ambient_git_environment_does_not_redefine_the_requested_scope() {
    use quecto::application::sessions::ports::session_home::WorkspaceDiscovery;
    use quecto::domain::session_home::WorkspaceGroup;
    use quecto::infrastructure::workspace::git_scope_discovery::GitScopeDiscovery;
    use std::sync::{Mutex, OnceLock};
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let _env = ENV_LOCK.get_or_init(Mutex::default).lock().unwrap();

    let repo = tempfile::tempdir().unwrap();
    assert!(
        std::process::Command::new("git")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .arg("init")
            .arg("-q")
            .arg(repo.path())
            .output()
            .unwrap()
            .status
            .success()
    );
    let plain = tempfile::tempdir().unwrap();
    let discovery: &dyn WorkspaceDiscovery = &GitScopeDiscovery::default();
    let undisturbed = discovery.discover(plain.path()).unwrap();
    assert_eq!(
        undisturbed.group,
        WorkspaceGroup::Folder {
            directory: plain.path().canonicalize().unwrap()
        }
    );

    // The fixture helpers in this binary strip GIT_DIR/GIT_WORK_TREE from
    // their own git spawns, and the ceiling names only the plain folder,
    // which no other scenario walks through.
    // SAFETY: ENV_LOCK serialises this process-wide mutation.
    unsafe {
        std::env::set_var("GIT_DIR", repo.path().join(".git"));
        std::env::set_var("GIT_WORK_TREE", repo.path());
        std::env::set_var("GIT_CEILING_DIRECTORIES", plain.path());
    }
    let ambient = discovery.discover(plain.path());
    let in_repo = discovery.discover(repo.path());
    // SAFETY: as above; restored before the lock is released.
    unsafe {
        std::env::remove_var("GIT_DIR");
        std::env::remove_var("GIT_WORK_TREE");
        std::env::remove_var("GIT_CEILING_DIRECTORIES");
    }
    assert_eq!(
        ambient.unwrap(),
        undisturbed,
        "an exported GIT_DIR must not turn a plain folder into the repository"
    );
    assert!(
        matches!(in_repo.unwrap().group, WorkspaceGroup::Git { .. }),
        "the repository itself is still discovered from its own directory"
    );
}
