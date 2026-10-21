//! #2663: the id a board's records name it by.
use super::*;

/// #2663 review round 1: a board's id is its repository's: one id for
/// every spelling of one remote (scheme, userinfo, host case, `.git`, a
/// trailing `/`, the scp form), another for another repository.
#[test]
fn a_board_id_names_its_repository_whatever_its_spelling() {
    let id = |url: &str| GitBoardStore::board_id_of(url).unwrap();
    let same = [
        "https://github.com/o/r",
        "https://tok@github.com/o/r",
        "https://other@github.com/o/r",
        "https://user:secret@GitHub.com/o/r.git/",
        "ssh://git@github.com/o/r.git",
        "git@github.com:o/r.git",
    ];
    for url in same {
        assert_eq!(id(url), id(same[0]), "{url}");
        assert_eq!(committer::repository_of(url), "github.com/o/r", "{url}");
    }
    for other in [
        "https://github.com/o/s",
        "https://gitlab.com/o/r",
        "/tmp/remote.git",
    ] {
        assert_ne!(id(other), id(same[0]), "{other}");
    }
    assert_eq!(committer::repository_of("/tmp/remote.git/"), "/tmp/remote");
}
