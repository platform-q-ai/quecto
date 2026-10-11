use super::*;

#[test]
fn a_commit_id_is_full_lowercase_hex() {
    assert!(CommitId::parse(&"a".repeat(40)).is_ok());
    assert!(CommitId::parse(&"0".repeat(64)).is_ok());
    for odd in [
        "a".repeat(39),
        "a".repeat(41),
        "A".repeat(40),
        "g".repeat(40),
    ] {
        assert_eq!(CommitId::parse(&odd).unwrap_err().field, "commit", "{odd}");
    }
}

#[test]
fn a_branch_name_is_one_git_takes_as_it_is() {
    for name in [
        "main",
        "feat/2481-m1b",
        "a.b_c-d/e",
        "v1.2",
        "a_",
        &"x".repeat(MAX_BRANCH_BYTES),
    ] {
        assert!(BranchName::parse(name).is_ok(), "{name}");
    }
    for name in [
        "",
        "-rf",
        "/main",
        "main/",
        "a//b",
        "a..b",
        "x.lock",
        "a.lock/b",
        "a.",
        "a b",
        "ä",
        "a~1",
        "a:b",
        "HEAD",
        "refs/heads/x",
        &"a".repeat(40),
        &"f".repeat(64),
        &"x".repeat(MAX_BRANCH_BYTES + 1),
    ] {
        assert_eq!(
            BranchName::parse(name).unwrap_err().field,
            "branch",
            "{name:?}"
        );
    }
}
