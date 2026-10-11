use super::*;
use crate::domain::project_board::entities::fixtures_tests::{at, change, commit, person};

/// One change that must make valid fields invalid.
type Change_ = Box<dyn Fn(&mut ChangeFields)>;

fn fields() -> ChangeFields {
    change(1, Some(7)).into_fields()
}

#[test]
fn a_change_builds_linked_merged_or_unlinked() {
    let linked = Change::new(fields()).unwrap();
    assert!(linked.is_linked());
    let mut merged = fields();
    merged.merged = Some(commit('a'));
    assert!(Change::new(merged).is_ok());
    let mut unlinked = fields();
    unlinked.unlinked = Some((person("Ada"), at("09:00:00")));
    assert!(!Change::new(unlinked).unwrap().is_linked());
    let mut branch_only = fields();
    branch_only.pr = None;
    assert!(
        Change::new(branch_only).is_ok(),
        "a branch with no pull request yet"
    );
    for branch in [
        "main",
        "feat/2481-m1b",
        "a.b_c-d/e",
        &"x".repeat(MAX_BRANCH_BYTES),
    ] {
        let mut named = fields();
        named.branch = branch.into();
        assert!(Change::new(named).is_ok(), "{branch}");
    }
}

#[test]
fn fields_that_break_a_rule_build_no_change() {
    let cases: Vec<(&str, Change_)> = vec![
        ("number", Box::new(|c| c.number = 0)),
        ("number", Box::new(|c| c.number = MAX_CHANGE_NUMBER + 1)),
        ("updated", Box::new(|c| c.updated = at("08:00:00"))),
        ("pr", Box::new(|c| c.pr = Some(0))),
        ("merged", Box::new(|c| c.merged = Some("HEAD".into()))),
        ("merged", Box::new(|c| c.merged = Some("A".repeat(40)))),
        (
            "linked_by/email",
            Box::new(|c| c.linked_by.email = "ada".into()),
        ),
        (
            "unlinked",
            Box::new(|c| {
                c.merged = Some(commit('a'));
                c.unlinked = Some((person("Ada"), at("09:00:00")));
            }),
        ),
        (
            "unlinked/at",
            Box::new(|c| c.unlinked = Some((person("Ada"), at("08:00:00")))),
        ),
    ];
    for (field, change) in cases {
        let mut broken = fields();
        change(&mut broken);
        assert_eq!(Change::new(broken).unwrap_err().field, field);
    }
    for branch in [
        "", "-rf", "/main", "main/", "a//b", "a..b", "x.lock", "a b", "ä", "a~1", "a:b",
    ] {
        let mut named = fields();
        named.branch = branch.into();
        assert_eq!(
            Change::new(named).unwrap_err().field,
            "branch",
            "{branch:?}"
        );
    }
    let mut long = fields();
    long.branch = "x".repeat(MAX_BRANCH_BYTES + 1);
    assert_eq!(Change::new(long).unwrap_err().field, "branch");
    assert_eq!(fields().text_bytes(), "feat/b1".len());
}

#[test]
fn a_commit_id_is_full_lowercase_hex() {
    assert!(commit_id(&commit('a')));
    assert!(commit_id(&"0".repeat(64)));
    for odd in [
        "a".repeat(39),
        "a".repeat(41),
        "A".repeat(40),
        "g".repeat(40),
    ] {
        assert!(!commit_id(&odd), "{odd}");
    }
}
