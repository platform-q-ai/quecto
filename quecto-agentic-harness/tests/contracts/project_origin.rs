//! Contract for the `ProjectOrigin` port (swarm-container-matches-repo):
//! the launching project's `origin` remote, as git reads it from the
//! checkout's local configuration — the URL as written, `None` for a
//! directory that is no checkout or a checkout without `origin`.
use std::path::Path;
use std::process::Command;

use quecto::composition::container_configs::build_project_origin;

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()
        .expect("git runs");
    assert!(status.success(), "git {args:?}");
}

#[test]
fn a_checkouts_origin_is_its_url_as_written_from_anywhere_inside_it() {
    let checkout = tempfile::tempdir().unwrap();
    git(checkout.path(), &["init", "-q"]);
    git(
        checkout.path(),
        &[
            "remote",
            "add",
            "origin",
            "git@github.com:Platform-Q-AI/quecto-pocket.git",
        ],
    );
    let nested = checkout.path().join("src/deep");
    std::fs::create_dir_all(&nested).unwrap();
    let origins = build_project_origin();
    for dir in [checkout.path(), nested.as_path()] {
        assert_eq!(
            origins.origin(dir),
            Ok(Some(
                "git@github.com:Platform-Q-AI/quecto-pocket.git".to_string()
            )),
            "{}",
            dir.display()
        );
    }
}

#[test]
fn no_checkout_or_no_origin_is_none() {
    let plain = tempfile::tempdir().unwrap();
    let origins = build_project_origin();
    assert_eq!(origins.origin(plain.path()), Ok(None));
    let checkout = tempfile::tempdir().unwrap();
    git(checkout.path(), &["init", "-q"]);
    git(
        checkout.path(),
        &["remote", "add", "upstream", "https://github.com/o/r"],
    );
    assert_eq!(
        origins.origin(checkout.path()),
        Ok(None),
        "only origin counts"
    );
    assert_eq!(
        origins.origin(&plain.path().join("missing")),
        Ok(None),
        "a directory that is gone is no checkout"
    );
}
