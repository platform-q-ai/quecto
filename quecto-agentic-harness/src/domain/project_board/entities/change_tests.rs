use super::*;
use crate::domain::project_board::entities::fixtures_tests::{
    at, branch, change, commit, person, repo,
};

/// One change that must make valid fields invalid.
type Edit = Box<dyn Fn(&mut ChangeFields)>;

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
    let mut fork = fields();
    fork.head_repo = repo("someone/quecto");
    let fork = Change::new(fork).unwrap();
    assert_eq!(fork.head(), (&repo("someone/quecto"), &branch("feat/b1")));
}

/// Open is linked and not merged: the branch is still in use.
#[test]
fn a_change_is_open_while_linked_and_unmerged() {
    let mut merged = fields();
    merged.merged = Some(commit('a'));
    let mut unlinked = fields();
    unlinked.unlinked = Some((person("Ada"), at("09:00:00")));
    let open = [fields(), merged, unlinked].map(|fields| Change::new(fields).unwrap().is_open());
    assert_eq!(open, [true, false, false]);
}

#[test]
fn fields_that_break_a_rule_build_no_change() {
    let cases: Vec<(&str, Edit)> = vec![
        ("number", Box::new(|c| c.number = 0)),
        ("number", Box::new(|c| c.number = MAX_CHANGE_NUMBER + 1)),
        ("updated", Box::new(|c| c.updated = at("08:00:00"))),
        ("pr", Box::new(|c| c.pr = Some(0))),
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
        (
            "unlinked/at",
            Box::new(|c| c.unlinked = Some((person("Ada"), at("09:30:00")))),
        ),
        (
            "unlinked/by/email",
            Box::new(|c| {
                let mut by = person("Ada");
                by.email = "ada".into();
                c.unlinked = Some((by, at("09:00:00")));
            }),
        ),
    ];
    for (field, edit) in cases {
        let mut broken = fields();
        edit(&mut broken);
        assert_eq!(Change::new(broken).unwrap_err().field, field);
    }
}
