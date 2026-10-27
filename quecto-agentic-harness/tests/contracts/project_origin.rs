//! Contract for the `ProjectOrigin` port (swarm-container-matches-repo):
//! the launching project's `origin` remote, as `git remote get-url origin`
//! answers it — the URL with any `insteadOf` rewrite of the machine's
//! configuration applied, as the clone would see it — `None` for a
//! directory that is no checkout or a checkout without `origin`. The
//! checkouts are made by isolated git, and name a host no `insteadOf`
//! rewrites, so the machine's configuration cannot change the answer.
use quecto::composition::container_configs::build_project_origin;
use quecto::infrastructure::test_support::isolated_git::{checkout_with_origin, isolated_git_ok};

#[test]
fn a_checkouts_origin_is_its_url_as_written_from_anywhere_inside_it() {
    let root = tempfile::tempdir().unwrap();
    let origin = "git@git.example.test:Platform-Q-AI/quecto-pocket.git";
    let checkout = checkout_with_origin(root.path(), origin);
    let nested = checkout.join("src/deep");
    std::fs::create_dir_all(&nested).unwrap();
    let origins = build_project_origin();
    for dir in [checkout.as_path(), nested.as_path()] {
        assert_eq!(
            origins.origin(dir),
            Ok(Some(origin.to_string())),
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
    isolated_git_ok(plain.path(), checkout.path(), &["init", "-q"]);
    isolated_git_ok(
        plain.path(),
        checkout.path(),
        &["remote", "add", "upstream", "https://git.example.test/o/r"],
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
