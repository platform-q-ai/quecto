//! `CheckoutHistory` on the git adapter (#2687): a commit is an ancestor of
//! itself and of its descendants, and every other answer (a commit outside
//! the history, the reverse order, an unknown commit, text that is not a
//! full commit id, a directory that is no repository) is no.
use std::path::Path;
use std::process::Command;

use quecto::application::swarm::ports::board_files::CheckoutHistory;
use quecto::infrastructure::workspace::checkout_history::GitCheckoutHistory;

/// git in `dir` with no user or system configuration and no inherited git
/// environment: its trimmed output.
fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=Test", "-c", "user.email=t@example.invalid"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        // Only PATH passes: a git hook's GIT_DIR or GIT_INDEX_FILE (the
        // pre-push gate runs these tests) must never reach the real
        // repository.
        .env_clear()
        .envs(std::env::var_os("PATH").map(|path| ("PATH", path)))
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("git runs");
    assert!(output.status.success(), "git {args:?}");
    String::from_utf8(output.stdout)
        .expect("git prints text")
        .trim_end()
        .to_owned()
}

#[test]
fn only_a_confirmed_ancestor_is_one() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    git(root, &["commit", "-q", "--allow-empty", "-m", "base"]);
    let base = git(root, &["rev-parse", "HEAD"]);
    git(root, &["commit", "-q", "--allow-empty", "-m", "next"]);
    let next = git(root, &["rev-parse", "HEAD"]);
    let tree = git(root, &["rev-parse", "HEAD^{tree}"]);
    let unrelated = git(root, &["commit-tree", &tree, "-m", "unrelated"]);
    let unknown = "0123456789abcdef0123456789abcdef01234567";
    let history = GitCheckoutHistory::new(root);
    for (ancestor, descendant) in [(&base, &next), (&base, &base), (&next, &next)] {
        assert!(
            history.is_ancestor(ancestor, descendant),
            "{ancestor} {descendant}"
        );
    }
    let short = &base[..12];
    for (ancestor, descendant) in [
        (next.as_str(), base.as_str()),
        (base.as_str(), unrelated.as_str()),
        (unknown, next.as_str()),
        (base.as_str(), unknown),
        (short, next.as_str()),
        ("HEAD~1", "HEAD"),
        ("--all", next.as_str()),
    ] {
        assert!(
            !history.is_ancestor(ancestor, descendant),
            "{ancestor} {descendant}"
        );
    }
    let elsewhere = tempfile::tempdir().unwrap();
    assert!(!GitCheckoutHistory::new(elsewhere.path()).is_ancestor(&base, &next));
}
