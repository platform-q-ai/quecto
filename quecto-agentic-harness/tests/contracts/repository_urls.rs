//! Contract for the `RepositoryUrls` port (swarm-container-matches-repo):
//! a remote URL read as the repository it names — host, owner and name,
//! case-insensitive — in every form git clones a hosted repository over,
//! and nothing for a URL naming none.
use quecto::composition::container_configs::build_repository_urls;

#[test]
fn every_form_of_one_repository_names_the_same_repository() {
    let urls = build_repository_urls();
    let canonical = urls
        .identity("https://github.com/platform-q-ai/quecto")
        .expect("a repository");
    assert_eq!(canonical.to_string(), "platform-q-ai/quecto");
    for url in [
        "https://github.com/Platform-Q-AI/Quecto.git/",
        "git@github.com:platform-q-ai/quecto.git",
        "ssh://git@github.com:22/platform-q-ai/quecto",
        "https://token@github.com/platform-q-ai/quecto",
    ] {
        assert_eq!(urls.identity(url).as_ref(), Some(&canonical), "{url}");
    }
    assert_ne!(
        urls.identity("https://github.com/platform-q-ai/quecto-pocket"),
        Some(canonical.clone())
    );
    assert_ne!(
        urls.identity("https://gitlab.com/platform-q-ai/quecto"),
        Some(canonical)
    );
}

#[test]
fn a_url_naming_no_hosted_repository_is_none() {
    let urls = build_repository_urls();
    for url in [
        "",
        "/srv/git/quecto.git",
        "file:///srv/git/quecto.git",
        "https://github.com/platform-q-ai",
        "https://gitlab.com/group/subgroup/quecto",
    ] {
        assert_eq!(urls.identity(url), None, "{url:?}");
    }
}
