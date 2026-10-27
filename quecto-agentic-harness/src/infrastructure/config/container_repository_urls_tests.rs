use super::{RemoteRepositoryUrls, repository_identity};
use crate::application::subagents::ports::container_repository::RepositoryUrls;
use crate::domain::environments::entities::repository_identity::RepositoryIdentity;

fn quecto() -> RepositoryIdentity {
    RepositoryIdentity::new("github.com", "platform-q-ai", "quecto").unwrap()
}

#[test]
fn every_spelling_of_one_github_repository_is_that_repository() {
    for url in [
        "https://github.com/platform-q-ai/quecto",
        "https://github.com/platform-q-ai/quecto.git",
        "https://github.com/platform-q-ai/quecto/",
        "https://github.com/platform-q-ai/quecto.git/",
        "https://github.com/platform-q-ai/quecto//",
        "https://GitHub.com/Platform-Q-AI/Quecto",
        "https://github.com/platform-q-ai/Quecto.GIT",
        "http://github.com/platform-q-ai/quecto",
        "https://x-access-token:ghp_secret@github.com/platform-q-ai/quecto.git",
        "https://github.com:443/platform-q-ai/quecto",
        "ssh://git@github.com/platform-q-ai/quecto.git",
        "ssh://git@github.com:22/platform-q-ai/quecto",
        "git+ssh://git@github.com/platform-q-ai/quecto",
        "git://github.com/platform-q-ai/quecto.git",
        "git@github.com:platform-q-ai/quecto.git",
        "git@github.com:platform-q-ai/quecto",
        "git@github.com:/platform-q-ai/quecto/",
        "github.com:platform-q-ai/quecto",
        "  https://github.com/platform-q-ai/quecto\n",
    ] {
        assert_eq!(repository_identity(url), Some(quecto()), "{url}");
        assert_eq!(RemoteRepositoryUrls.identity(url), Some(quecto()), "{url}");
    }
}

#[test]
fn another_owner_name_or_host_is_another_repository() {
    for url in [
        "https://github.com/platform-q-ai/quecto-pocket",
        "git@github.com:someone/quecto.git",
        "https://gitlab.com/platform-q-ai/quecto",
    ] {
        let identity = repository_identity(url).expect("a readable URL");
        assert_ne!(identity, quecto(), "{url}");
    }
}

#[test]
fn a_url_naming_no_repository_by_host_owner_and_name_is_unreadable() {
    for url in [
        "",
        "   ",
        "/srv/git/quecto.git",
        "../quecto",
        "file:///srv/git/quecto.git",
        "file://localhost/srv/git/quecto",
        "https://github.com/platform-q-ai",
        "https://github.com/platform-q-ai/quecto/tree/main",
        "https://gitlab.com/group/subgroup/quecto",
        "https://github.com/",
        "https://github.com",
        "https:///platform-q-ai/quecto",
        "https://github.com/platform-q-ai/quecto?ref=main",
        "https://github.com/platform-q-ai/quecto#readme",
        "https://github.com:abc/platform-q-ai/quecto",
        "ftp://github.com/platform-q-ai/quecto",
        "C:/repos/quecto",
        "git@github.com:platform-q-ai",
        "git@:platform-q-ai/quecto",
        "https://github.com/platform-q-ai/.git",
    ] {
        assert_eq!(repository_identity(url), None, "{url:?}");
    }
}
