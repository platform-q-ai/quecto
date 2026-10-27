use super::RepositoryIdentity;

fn identity(host: &str, owner: &str, name: &str) -> RepositoryIdentity {
    RepositoryIdentity::new(host, owner, name).expect("a valid identity")
}

#[test]
fn an_identity_is_the_same_repository_whatever_the_case_of_its_parts() {
    assert_eq!(
        identity("GitHub.com", "Platform-Q-AI", "Quecto"),
        identity("github.com", "platform-q-ai", "quecto")
    );
}

#[test]
fn identities_differ_by_owner_name_or_host() {
    let quecto = identity("github.com", "platform-q-ai", "quecto");
    assert_ne!(
        quecto,
        identity("github.com", "platform-q-ai", "quecto-pocket")
    );
    assert_ne!(quecto, identity("github.com", "someone-else", "quecto"));
    assert_ne!(quecto, identity("gitlab.com", "platform-q-ai", "quecto"));
}

#[test]
fn a_github_identity_reads_as_owner_slash_name_and_another_host_keeps_its_host() {
    let github = identity("github.com", "Platform-Q-AI", "Quecto");
    assert_eq!(github.to_string(), "platform-q-ai/quecto");
    assert_eq!(
        github.https_url(),
        "https://github.com/platform-q-ai/quecto"
    );
    let other = identity("git.example.com", "team", "tool");
    assert_eq!(other.to_string(), "git.example.com/team/tool");
    assert_eq!(other.https_url(), "https://git.example.com/team/tool");
}

#[test]
fn only_parts_of_the_allowed_shape_make_an_identity() {
    for (host, owner, name) in [
        ("", "o", "n"),
        ("github.com", "", "n"),
        ("github.com", "o", ""),
        ("github.com", "o", "."),
        ("github.com", "o", ".."),
        ("github.com", "-o", "n"),
        ("github.com", "o", "-n"),
        ("github.com", "o/x", "n"),
        ("github.com", "o", "n x"),
        ("github.com", "o", "n@x"),
        ("git hub.com", "o", "n"),
        ("-github.com", "o", "n"),
        ("github.com:22", "o", "n"),
        ("github.com", "ö", "n"),
        ("localhost", "o", "n"),
        ("c", "repos", "quecto"),
    ] {
        assert!(
            RepositoryIdentity::new(host, owner, name).is_none(),
            "{host} {owner} {name} must be refused"
        );
    }
}

#[test]
fn an_overlong_part_is_refused() {
    let long = "a".repeat(257);
    assert!(RepositoryIdentity::new("github.com", &long, "n").is_none());
    assert!(RepositoryIdentity::new("github.com", "o", &long).is_none());
    assert!(RepositoryIdentity::new(&long, "o", "n").is_none());
}
