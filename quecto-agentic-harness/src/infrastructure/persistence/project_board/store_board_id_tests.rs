//! #2663: the id a board's records name it by.
use super::*;

/// #2663 review round 1: a board's id is its repository's: one id for
/// every spelling of one remote (scheme, userinfo, host case, `.git`, a
/// trailing `/`, the scp form), another for another repository.
#[test]
fn a_board_id_names_its_repository_whatever_its_spelling() {
    let here = std::path::Path::new("/nowhere/clone");
    let id = |url: &str| GitBoardStore::board_id_of(url, here).unwrap();
    let same = [
        "https://github.com/o/r",
        "https://tok@github.com/o/r",
        "https://other@github.com/o/r",
        "https://user:secret@GitHub.com/o/r.git/",
        "ssh://git@github.com/o/r.git",
        "git@github.com:o/r.git",
        "ssh://git@github.com:22/o/r.git",
        "https://github.com:443/o/r",
    ];
    for url in same {
        assert_eq!(id(url), id(same[0]), "{url}");
        assert_eq!(
            committer::repository_of(url, here),
            "github.com/o/r",
            "{url}"
        );
    }
    for other in [
        "https://github.com/o/s",
        "https://gitlab.com/o/r",
        "/tmp/remote.git",
    ] {
        assert_ne!(id(other), id(same[0]), "{other}");
    }
    assert_eq!(
        committer::repository_of("/nowhere/remote.git/", here),
        "/nowhere/remote"
    );
    // A port that is not the scheme's default is kept.
    assert_ne!(id("ssh://git@github.com:2222/o/r"), id(same[0]));
}

/// #2663 review round 2: a relative local remote is the absolute path it
/// names from the clone, so two clones' `../board.git` never collide, nor
/// does one with a host's `github.com/o/r`.
#[test]
fn a_relative_local_remote_is_resolved_from_its_clone() {
    let dir = tempfile::tempdir().unwrap();
    let (one, two) = (dir.path().join("a/clone"), dir.path().join("b/clone"));
    for path in [&one, &two] {
        std::fs::create_dir_all(path).unwrap();
        std::fs::create_dir_all(path.join("../board.git")).unwrap();
    }
    let id = |url: &str, repo: &std::path::Path| GitBoardStore::board_id_of(url, repo).unwrap();
    assert_ne!(id("../board.git", &one), id("../board.git", &two));
    assert_eq!(id("../board.git", &one), id("../board.git/", &one));
    let absolute = dir.path().join("a/board.git");
    assert_eq!(
        id("../board.git", &one),
        id(absolute.to_str().unwrap(), &two),
        "the same repository from anywhere"
    );
    assert_ne!(
        id("github.com/o/r", &one),
        id("https://github.com/o/r", &one)
    );
}
