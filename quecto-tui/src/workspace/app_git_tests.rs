//! Tests for git branch footer discovery and polling policy.

#[tokio::test]
async fn git_branch_refresh_task_reflects_branch_switches_promptly() {
    let repo = std::env::temp_dir().join(format!(
        "quecto-tui-branch-refresh-test-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("unnamed")
    ));
    let _ = std::fs::remove_dir_all(&repo);
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();

    let mut h = super::tui_harness::TuiHarness::new().await;
    h.set_git_repo(repo.clone());
    assert!(h.apply_branch(Some("main".to_string())));

    std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/f\n").unwrap();
    assert!(h.refresh_branch_from_repo().await);
    assert!(
        h.bottom_stack().contains("(f)"),
        "production branch refresh path should update the footer after a branch switch"
    );
    let interval = super::app_git::GIT_BRANCH_POLL_INTERVAL;
    assert!(
        interval >= std::time::Duration::from_secs(1),
        "branch polling must not perform sub-second periodic work while idle (#978), got {interval:?}"
    );
    assert!(
        interval <= std::time::Duration::from_secs(5),
        "branch switches must still surface within a few seconds, got {interval:?}"
    );

    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn read_git_branch_reflects_head_changes_without_restart() {
    let repo = std::env::temp_dir().join(format!(
        "quecto-tui-branch-test-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("unnamed")
    ));
    let _ = std::fs::remove_dir_all(&repo);
    std::fs::create_dir_all(repo.join(".git")).unwrap();

    std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    assert_eq!(
        super::app_git::read_git_branch_from(&repo),
        Some("main".to_string())
    );

    std::fs::write(
        repo.join(".git/HEAD"),
        "ref: refs/heads/feature/footer-branch\n",
    )
    .unwrap();
    assert_eq!(
        super::app_git::read_git_branch_from(&repo),
        Some("feature/footer-branch".to_string())
    );

    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn read_git_branch_strips_control_and_bidi_sequences_from_head() {
    let repo = std::env::temp_dir().join(format!(
        "quecto-tui-branch-sanitize-test-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("unnamed")
    ));
    let _ = std::fs::remove_dir_all(&repo);
    std::fs::create_dir_all(repo.join(".git")).unwrap();

    std::fs::write(
        repo.join(".git/HEAD"),
        "ref: refs/heads/feature/\x1b]0;owned\x07foot\u{202e}er\n",
    )
    .unwrap();
    assert_eq!(
        super::app_git::read_git_branch_from(&repo),
        Some("feature/]0;ownedfooter".to_string())
    );

    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn read_git_branch_resolves_gitdir_file_from_subdirectory() {
    let repo = std::env::temp_dir().join(format!(
        "quecto-tui-branch-gitdir-test-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("unnamed")
    ));
    let _ = std::fs::remove_dir_all(&repo);
    let worktree = repo.join("worktree");
    let gitdir = repo.join("actual-git-dir");
    let child = worktree.join("nested/child");
    std::fs::create_dir_all(&child).unwrap();
    std::fs::create_dir_all(&gitdir).unwrap();
    std::fs::write(worktree.join(".git"), "gitdir: ../actual-git-dir\n").unwrap();
    std::fs::write(gitdir.join("HEAD"), "ref: refs/heads/worktree/topic\n").unwrap();

    assert_eq!(
        super::app_git::read_git_branch_from(&child),
        Some("worktree/topic".to_string())
    );

    let _ = std::fs::remove_dir_all(&repo);
}

#[cfg(unix)]
#[test]
fn read_git_branch_rejects_head_symlink() {
    let repo = std::env::temp_dir().join(format!(
        "quecto-tui-branch-symlink-test-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("unnamed")
    ));
    let _ = std::fs::remove_dir_all(&repo);
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    let target = repo.join("target-head");
    std::fs::write(&target, "ref: refs/heads/main\n").unwrap();
    std::os::unix::fs::symlink(&target, repo.join(".git/HEAD")).unwrap();

    assert_eq!(super::app_git::read_git_branch_from(&repo), None);

    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn read_git_branch_accepts_head_at_size_limit() {
    let repo = std::env::temp_dir().join(format!(
        "quecto-tui-branch-head-limit-test-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("unnamed")
    ));
    let _ = std::fs::remove_dir_all(&repo);
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    let prefix = "ref: refs/heads/";
    let suffix = "\n";
    let branch_len = super::app_git::GIT_HEAD_READ_LIMIT as usize - prefix.len() - suffix.len();
    let branch = "x".repeat(branch_len);
    std::fs::write(repo.join(".git/HEAD"), format!("{prefix}{branch}{suffix}")).unwrap();

    assert_eq!(super::app_git::read_git_branch_from(&repo), Some(branch));

    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn read_git_branch_rejects_oversized_head() {
    let repo = std::env::temp_dir().join(format!(
        "quecto-tui-branch-oversized-test-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("unnamed")
    ));
    let _ = std::fs::remove_dir_all(&repo);
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    let prefix = "ref: refs/heads/";
    let suffix = "\n";
    let branch_len = super::app_git::GIT_HEAD_READ_LIMIT as usize + 1 - prefix.len() - suffix.len();
    let branch = "x".repeat(branch_len);
    std::fs::write(repo.join(".git/HEAD"), format!("{prefix}{branch}{suffix}")).unwrap();

    assert_eq!(super::app_git::read_git_branch_from(&repo), None);

    let _ = std::fs::remove_dir_all(&repo);
}
