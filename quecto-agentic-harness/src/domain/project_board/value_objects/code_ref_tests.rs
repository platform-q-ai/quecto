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

/// Every name here was checked with `git check-ref-format --branch`
/// (git 2.55): the names it takes, and the names it refuses.
#[test]
fn a_branch_name_is_one_git_takes() {
    for name in [
        "main",
        "feat/2481-m1b",
        "a.b_c-d/e",
        "v1.2",
        "_a",
        "a/-b",
        "a@b",
        "@a",
        "a@",
        "a/@",
        "a#b+c,d=e",
        "a{b}",
        "a'b\"c!d$e%f&g(h)i<j>k;l]m_n|o",
        "|",
        "a.lock.b",
        "a./b",
        "+",
        &"x".repeat(MAX_BRANCH_BYTES),
    ] {
        assert!(BranchName::parse(name).is_ok(), "{name}");
    }
    for name in [
        "", "-rf", "/main", "main/", "a//b", "a..b", "aa..", ".a", "a/.b", "x.lock", "a.lock/b",
        "a/b.lock", "a.", "a/b.", "a@{b", "a b", "a\tb", "a\u{7f}b", "a~1", "a^b", "a:b", "a?b",
        "a*b", "a[b", "a\\b", // No board file holds a backtick: a code span's edge.
        "a`b", "`",
    ] {
        assert_eq!(
            BranchName::parse(name).unwrap_err().field,
            "branch",
            "{name:?}"
        );
    }
}

/// Names git takes that the board refuses: names git or a reader would
/// take for something else, names past printable ASCII, and the length
/// git does not bound.
#[test]
fn a_branch_name_is_not_one_git_reads_as_something_else() {
    for name in [
        "ä",
        "@",
        "HEAD",
        "FETCH_HEAD",
        "ORIG_HEAD",
        "MERGE_HEAD",
        "refs/heads/x",
        "refs",
        "heads/x",
        "tags/v1",
        "remotes/origin/main",
        &"a".repeat(40),
        &"f".repeat(64),
        &"A".repeat(40),
        &"aB".repeat(32),
        &"x".repeat(MAX_BRANCH_BYTES + 1),
    ] {
        assert_eq!(
            BranchName::parse(name).unwrap_err().field,
            "branch",
            "{name:?}"
        );
    }
    assert!(BranchName::parse("head").is_ok(), "git's names are exact");
    for name in ["feat/heads", "tagsx/y", "headsx", "@a"] {
        assert!(BranchName::parse(name).is_ok(), "{name}");
    }
    assert!(BranchName::parse(&"a".repeat(41)).is_ok());
}

#[test]
fn a_repository_is_named_owner_slash_name() {
    let longest = format!(
        "{}/{}",
        "o".repeat(MAX_OWNER_BYTES),
        "n".repeat(MAX_REPO_PART_BYTES)
    );
    for name in [
        "platform-q-ai/quecto",
        "a/.github",
        "a-b/c.d-e",
        "A1/__init__",
        "a/www.site",
        "a--b/c",
        // An old account may end with a hyphen; an enterprise-managed
        // user's owner is a name and one `_shortcode`.
        "a-/b",
        "ada_corp/x",
        "ada-b_x1/x",
        &longest,
    ] {
        assert!(RepoName::parse(name).is_ok(), "{name}");
    }
    let too_long = format!("o/{}", "n".repeat(MAX_REPO_PART_BYTES + 1));
    let owner_too_long = format!("{}/n", "o".repeat(40));
    for name in [
        "",
        "quecto",
        "a/b/c",
        "/b",
        "a/",
        "./b",
        "a/..",
        "a/.",
        "a b/c",
        "a/b~",
        "ä/b",
        &too_long,
        &owner_too_long,
        // GitHub's owners: letters, digits and hyphens, not first, then at
        // most one `_shortcode`; a name never starts with a hyphen.
        "_a/b",
        "a_/b",
        "a__b/c",
        "a_b_c/d",
        "a_-b/c",
        "a./b",
        ".a/b",
        "-a/b",
        "-/b",
        "a.b/c",
        "a/b|c",
        "a/b`c",
        "a/-b",
    ] {
        assert_eq!(RepoName::parse(name).unwrap_err().field, "repo", "{name:?}");
    }
}

/// The host names repositories without regard to case: names that differ
/// only in case are one repository, shown as given.
#[test]
fn a_repository_is_the_same_whatever_its_case() {
    let upper = RepoName::parse("Platform-Q-AI/Quecto").unwrap();
    let lower = RepoName::parse("platform-q-ai/quecto").unwrap();
    assert_eq!(upper, lower);
    assert_eq!(upper.cmp(&lower), std::cmp::Ordering::Equal);
    let hashes = std::collections::HashSet::from([upper.clone(), lower]);
    assert_eq!(hashes.len(), 1);
    assert_eq!(upper.as_str(), "Platform-Q-AI/Quecto");
    assert_ne!(upper, RepoName::parse("platform-q-ai/quecto2").unwrap());
}

/// GitHub's owner limit, pinned: 39 bytes before any `_shortcode`.
#[test]
fn an_owner_is_at_most_39_bytes_before_its_shortcode() {
    assert_eq!(MAX_OWNER_BYTES, 39);
    let owner = "o".repeat(39);
    assert!(RepoName::parse(&format!("{owner}/n")).is_ok());
    assert!(RepoName::parse(&format!("{owner}_corp/n")).is_ok());
    assert!(RepoName::parse(&format!("{owner}o/n")).is_err());
}

/// A refusal says why: a branch names the characters a board file cannot
/// hold, a repository GitHub's owner grammar. The field codes stay.
#[test]
fn a_refused_name_says_why() {
    let branch = BranchName::parse("a`b").unwrap_err();
    assert_eq!(branch.field, "branch");
    assert!(branch.problem.contains("backtick"), "{}", branch.problem);
    let repo = RepoName::parse("a_/b").unwrap_err();
    assert_eq!(repo.field, "repo");
    assert!(
        repo.problem.contains("owner") && repo.problem.contains("_shortcode"),
        "{}",
        repo.problem
    );
}
