use super::*;

fn init_git_repo(path: &Path) {
    std::fs::create_dir_all(path).unwrap();
    let st = Command::new("git").arg("init").arg(path).status().unwrap();
    assert!(st.success());

    let readme = path.join("README.md");
    std::fs::write(&readme, "hello\n").unwrap();
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(path)
            .arg("add")
            .arg("README.md")
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(path)
            .arg("-c")
            .arg("user.email=test@example.com")
            .arg("-c")
            .arg("user.name=test")
            .arg("commit")
            .arg("-m")
            .arg("init")
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(path)
            .arg("branch")
            .arg("-M")
            .arg("main")
            .status()
            .unwrap()
            .success()
    );
}

#[test]
fn test_repo_exists_and_ref_exists_inside_workspace() {
    let tmp = tempfile::TempDir::new().unwrap();
    let repo = tmp.path().join("repo-a");
    init_git_repo(&repo);

    let v = WorkspaceRepoValidator::new(tmp.path().to_path_buf());
    assert!(v.repo_exists("repo-a"));
    assert!(v.ref_exists("repo-a", "main"));
}

#[test]
fn test_repo_outside_workspace_rejected() {
    let ws = tempfile::TempDir::new().unwrap();
    let outside = tempfile::TempDir::new().unwrap();
    let repo = outside.path().join("repo-b");
    init_git_repo(&repo);

    let v = WorkspaceRepoValidator::new(ws.path().to_path_buf());
    assert!(!v.repo_exists(repo.to_str().unwrap()));
}

#[test]
fn test_repo_with_gitdir_outside_workspace_rejected() {
    let ws = tempfile::TempDir::new().unwrap();
    let outside = tempfile::TempDir::new().unwrap();
    let outside_repo = outside.path().join("outside-repo");
    init_git_repo(&outside_repo);

    let fake_repo = ws.path().join("fake-repo");
    std::fs::create_dir_all(&fake_repo).unwrap();
    std::fs::write(
        fake_repo.join(".git"),
        format!("gitdir: {}\n", outside_repo.join(".git").display()),
    )
    .unwrap();

    let v = WorkspaceRepoValidator::new(ws.path().to_path_buf());
    assert!(!v.repo_exists("fake-repo"));
}

#[test]
fn test_ref_with_option_like_prefix_rejected() {
    let tmp = tempfile::TempDir::new().unwrap();
    let repo = tmp.path().join("repo-a");
    init_git_repo(&repo);

    let v = WorkspaceRepoValidator::new(tmp.path().to_path_buf());
    assert!(!v.ref_exists("repo-a", "--help"));
}

#[test]
fn test_skill_exists_checks_skill_md() {
    let tmp = tempfile::TempDir::new().unwrap();
    let skill_dir = tmp.path().join("skills").join("default-skill");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: default-skill\ndescription: d\n---\nX",
    )
    .unwrap();

    let r = WorkspaceSkillResolver::new(tmp.path().to_path_buf());
    assert!(r.skill_exists("default-skill"));
    assert!(!r.skill_exists("missing"));
}

#[test]
fn test_skill_invalid_name_rejected() {
    let tmp = tempfile::TempDir::new().unwrap();
    let r = WorkspaceSkillResolver::new(tmp.path().to_path_buf());
    assert!(!r.skill_exists("../escape"));
}

#[cfg(unix)]
#[test]
fn test_skill_symlink_outside_workspace_rejected() {
    use std::os::unix::fs::symlink;

    let ws = tempfile::TempDir::new().unwrap();
    let outside = tempfile::TempDir::new().unwrap();

    let outside_skill = outside.path().join("outside-skill");
    std::fs::create_dir_all(&outside_skill).unwrap();
    std::fs::write(outside_skill.join("SKILL.md"), "x").unwrap();

    let skills_dir = ws.path().join("skills");
    std::fs::create_dir_all(&skills_dir).unwrap();
    symlink(&outside_skill, skills_dir.join("default-skill")).unwrap();

    let r = WorkspaceSkillResolver::new(ws.path().to_path_buf());
    assert!(!r.skill_exists("default-skill"));
}

// ========================================================================
// is_safe_repo_name tests
// ========================================================================

#[test]
fn test_safe_repo_name_valid() {
    assert!(is_safe_repo_name("my-project"));
    assert!(is_safe_repo_name("project_123"));
    assert!(is_safe_repo_name("a"));
    assert!(is_safe_repo_name("repo.v2"));
}

#[test]
fn test_safe_repo_name_rejects_empty() {
    assert!(!is_safe_repo_name(""));
}

#[test]
fn test_safe_repo_name_rejects_traversal() {
    assert!(!is_safe_repo_name(".."));
    assert!(!is_safe_repo_name("a/b"));
    assert!(!is_safe_repo_name("a\\b"));
}

#[test]
fn test_safe_repo_name_rejects_leading_dot_or_dash() {
    assert!(!is_safe_repo_name(".hidden"));
    assert!(!is_safe_repo_name("-flag"));
}

#[test]
fn test_safe_repo_name_rejects_too_long() {
    let long = "a".repeat(129);
    assert!(!is_safe_repo_name(&long));
    let exactly = "a".repeat(128);
    assert!(is_safe_repo_name(&exactly));
}

#[test]
fn test_safe_repo_name_rejects_special_chars() {
    assert!(!is_safe_repo_name("a b"));
    assert!(!is_safe_repo_name("a@b"));
    assert!(!is_safe_repo_name("a:b"));
}

// ========================================================================
// derive_name_from_url tests
// ========================================================================

#[test]
fn test_derive_name_https() {
    assert_eq!(
        derive_name_from_url("https://github.com/org/repo.git"),
        Some("repo".to_string())
    );
}

#[test]
fn test_derive_name_https_no_git_suffix() {
    assert_eq!(
        derive_name_from_url("https://github.com/org/repo"),
        Some("repo".to_string())
    );
}

#[test]
fn test_derive_name_ssh() {
    assert_eq!(
        derive_name_from_url("git@github.com:org/repo.git"),
        Some("repo".to_string())
    );
}

#[test]
fn test_derive_name_ssh_no_git_suffix() {
    assert_eq!(
        derive_name_from_url("git@github.com:org/my-project"),
        Some("my-project".to_string())
    );
}

#[test]
fn test_derive_name_invalid() {
    assert_eq!(derive_name_from_url("not-a-url"), None);
}

// ========================================================================
// is_safe_import_url tests
// ========================================================================

#[test]
fn test_safe_import_url_accepts_https() {
    assert!(is_safe_import_url("https://github.com/org/repo.git"));
}

#[test]
fn test_safe_import_url_accepts_ssh() {
    assert!(is_safe_import_url("ssh://git@github.com/org/repo"));
    assert!(is_safe_import_url("git@github.com:org/repo.git"));
}

#[test]
fn test_safe_import_url_rejects_ext() {
    assert!(!is_safe_import_url("ext::sh -c evil"));
}

#[test]
fn test_safe_import_url_rejects_file() {
    assert!(!is_safe_import_url("file:///tmp/repo"));
}

#[test]
fn test_safe_import_url_rejects_http() {
    assert!(!is_safe_import_url("http://example.com/repo"));
}

#[test]
fn test_safe_import_url_rejects_local_path() {
    assert!(!is_safe_import_url("/tmp/repo"));
}

#[test]
fn test_safe_import_url_rejects_git_protocol() {
    // git:// is unauthenticated/unencrypted — SSRF risk on port 9418
    assert!(!is_safe_import_url("git://host/repo"));
}

// ========================================================================
// WorkspaceRepoCreator tests
// ========================================================================

#[test]
fn test_creator_validate_name() {
    let tmp = tempfile::TempDir::new().unwrap();
    let c = WorkspaceRepoCreator::new(tmp.path().to_path_buf());
    assert!(c.validate_name("good-name").is_ok());
    assert!(c.validate_name("../escape").is_err());
    assert!(c.validate_name("").is_err());
}

#[test]
fn test_creator_exists() {
    let tmp = tempfile::TempDir::new().unwrap();
    let c = WorkspaceRepoCreator::new(tmp.path().to_path_buf());
    assert!(!c.exists("nonexistent"));
    std::fs::create_dir(tmp.path().join("exists")).unwrap();
    assert!(c.exists("exists"));
}

#[test]
fn test_creator_name_from_url() {
    let tmp = tempfile::TempDir::new().unwrap();
    let c = WorkspaceRepoCreator::new(tmp.path().to_path_buf());
    assert_eq!(
        c.name_from_url("https://github.com/org/repo.git").unwrap(),
        "repo"
    );
    assert!(c.name_from_url("bad").is_err());
}

#[test]
fn test_creator_create_repo() {
    let tmp = tempfile::TempDir::new().unwrap();
    let c = WorkspaceRepoCreator::new(tmp.path().to_path_buf());
    let path = c.create("test-proj", Some("A test project")).unwrap();
    assert!(PathBuf::from(&path).join(".git").exists());
    assert!(PathBuf::from(&path).join("README.md").exists());
    let readme = std::fs::read_to_string(PathBuf::from(&path).join("README.md")).unwrap();
    assert!(readme.contains("test-proj"));
    assert!(readme.contains("A test project"));

    // Verify the repo has a main branch with at least one commit
    let v = WorkspaceRepoValidator::new(tmp.path().to_path_buf());
    assert!(v.repo_exists("test-proj"));
    assert!(v.ref_exists("test-proj", "main"));
}

#[test]
fn test_creator_create_repo_no_description() {
    let tmp = tempfile::TempDir::new().unwrap();
    let c = WorkspaceRepoCreator::new(tmp.path().to_path_buf());
    let path = c.create("minimal", None).unwrap();
    let readme = std::fs::read_to_string(PathBuf::from(&path).join("README.md")).unwrap();
    assert_eq!(readme, "# minimal\n");
}

#[test]
fn test_creator_create_duplicate_rejected() {
    let tmp = tempfile::TempDir::new().unwrap();
    let c = WorkspaceRepoCreator::new(tmp.path().to_path_buf());
    c.create("dup", None).unwrap();
    // Second create should fail (create_dir fails on existing dir)
    let err = c.create("dup", None).unwrap_err();
    matches!(err, CommandError::GitFailed(_));
}

#[test]
fn test_creator_import_rejects_git_protocol() {
    let tmp = tempfile::TempDir::new().unwrap();
    let c = WorkspaceRepoCreator::new(tmp.path().to_path_buf());
    let err = c.import("git://host/repo", "repo").unwrap_err();
    assert_eq!(err, CommandError::InvalidUrl);
}

#[test]
fn test_sanitize_git_error_caps_length() {
    let long = "a".repeat(500);
    let result = sanitize_git_error(&long);
    assert_eq!(result.len(), 256);
}

#[test]
fn test_sanitize_git_error_empty() {
    assert_eq!(sanitize_git_error(""), "git operation failed");
    assert_eq!(sanitize_git_error("   "), "git operation failed");
}
