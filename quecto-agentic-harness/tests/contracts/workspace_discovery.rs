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
