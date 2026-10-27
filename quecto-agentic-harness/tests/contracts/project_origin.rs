//! Contract for the `ProjectOrigin` port (swarm-container-matches-repo):
//! the launching project's `origin` remote as written in the checkout's
//! own configuration (`remote.origin.url`), no `insteadOf` applied, so it
//! compares like for like with a container config's `--repo`, which is
//! read as written too; `None` for a directory that is no checkout or a
//! checkout without `origin`. The checkouts are made by isolated git.
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

/// An ssh alias host with an `insteadOf` rewrite configured is read as
/// written, not rewritten: the container's `--repo`, written the same way,
/// is the same repository.
#[test]
fn an_insteadof_alias_is_read_as_written() {
    let root = tempfile::tempdir().unwrap();
    let alias = "git@github-work:platform-q-ai/quecto-pocket.git";
    let checkout = checkout_with_origin(root.path(), alias);
    isolated_git_ok(
        root.path(),
        &checkout,
        &[
            "config",
            "url.https://rewritten.example.test/.insteadOf",
            "git@github-work:",
        ],
    );
    assert_eq!(
        build_project_origin().origin(&checkout),
        Ok(Some(alias.to_string()))
    );
    let urls = quecto::composition::container_configs::build_repository_urls();
    assert_eq!(
        urls.identity(alias),
        urls.identity("git@GitHub-Work:Platform-Q-AI/quecto-pocket")
    );
    assert!(urls.identity(alias).is_some());
}
