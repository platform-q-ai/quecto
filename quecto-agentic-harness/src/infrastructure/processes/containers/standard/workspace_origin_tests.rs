use super::GitWorkspaceOrigin;
use crate::application::environments::ports::WorkspaceOrigin;

fn git(dir: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

#[test]
fn the_origin_url_is_read_and_its_absence_is_none() {
    let dir = tempfile::TempDir::new().unwrap();
    assert_eq!(GitWorkspaceOrigin.origin(dir.path()).unwrap(), None);
    git(dir.path(), &["init", "-q"]);
    assert_eq!(GitWorkspaceOrigin.origin(dir.path()).unwrap(), None);
    git(
        dir.path(),
        &["remote", "add", "origin", "https://example.test/o.git"],
    );
    assert_eq!(
        GitWorkspaceOrigin.origin(dir.path()).unwrap().as_deref(),
        Some("https://example.test/o.git")
    );
    let missing = dir.path().join("nope");
    assert_eq!(GitWorkspaceOrigin.origin(&missing).unwrap(), None);
}

#[test]
fn the_toplevel_is_the_checkout_root_from_anywhere_below_it_and_none_outside_one() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    assert_eq!(GitWorkspaceOrigin.toplevel(&root).unwrap(), None);
    git(&root, &["init", "-q"]);
    assert_eq!(
        GitWorkspaceOrigin.toplevel(&root).unwrap().as_deref(),
        Some(root.as_path())
    );
    let below = root.join("a/b");
    std::fs::create_dir_all(&below).unwrap();
    assert_eq!(
        GitWorkspaceOrigin.toplevel(&below).unwrap().as_deref(),
        Some(root.as_path())
    );
    let missing = root.join("nope");
    assert_eq!(GitWorkspaceOrigin.toplevel(&missing).unwrap(), None);
}
