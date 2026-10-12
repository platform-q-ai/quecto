use super::*;
use crate::domain::project_board::entities::change::Change;
use crate::domain::project_board::services::fixtures_tests::{change, task, without_prs};
use crate::domain::project_board::value_objects::reference::{IssueNumber, Reference};
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;

fn slug(id: &str) -> Slug {
    Slug::parse(id).unwrap()
}

/// Task `id`, with issue `issue`, the old-way pull requests `prs` and the
/// references `refs`.
fn with(id: &str, issue: Option<u64>, prs: &[u64], refs: &[&str]) -> Task {
    let mut fields = without_prs(task(TaskStatus::Ready)).into_fields();
    fields.id = slug(id);
    fields.github_issue = issue.map(|issue| IssueNumber::new(issue).unwrap());
    fields.prs = prs.to_vec();
    fields.references = refs
        .iter()
        .map(|text| Reference::parse(text).unwrap())
        .collect();
    Task::new(fields).unwrap()
}

/// Change `number` of task `id`, linking pull request `pr`.
fn of(id: &str, number: u32, pr: u64) -> Change {
    let mut fields = change(number, Some(pr)).into_fields();
    fields.task = slug(id);
    Change::new(fields).unwrap()
}

#[test]
fn a_board_whose_numbers_agree_holds() {
    let board = [
        with("a", Some(1), &[10], &["#11"]),
        with("b", Some(2), &[], &[]),
        with("c", None, &[11], &[]),
    ];
    assert_eq!(number_clashes(&board, &[of("c", 1, 12)]), []);
    assert_eq!(issue_numbers_hold(&board, &[of("c", 1, 12)]), Ok(()));
    assert_eq!(number_clashes(&[], &[]), []);
}

/// Every clash is found, each naming both sides: two tasks with one issue,
/// an issue that is a pull request (linked the old way or by a change),
/// and a task naming by `#n` the pull request of its own change.
#[test]
fn every_clash_is_found_naming_both_sides() {
    let board = [
        with("a", Some(1), &[], &[]),
        with("b", Some(1), &[], &[]),
        with("c", Some(11), &[], &[]),
        with("d", None, &[11], &[]),
        with("e", Some(12), &[], &["#13"]),
    ];
    let changes = [of("f", 1, 12), of("e", 2, 13)];
    assert_eq!(
        number_clashes(&board, &changes),
        [
            BoardRuleError::IssueShared {
                issue: 1,
                tasks: [slug("a"), slug("b")]
            },
            BoardRuleError::IssueIsPr {
                issue: 11,
                task: slug("c"),
                pr_holder: slug("d")
            },
            BoardRuleError::IssueIsPr {
                issue: 12,
                task: slug("e"),
                pr_holder: slug("f")
            },
            BoardRuleError::OwnPrReferenced {
                task: slug("e"),
                pr: 13,
                change: 2
            },
        ]
    );
    assert_eq!(
        issue_numbers_hold(&board, &changes),
        Err(BoardRuleError::IssueShared {
            issue: 1,
            tasks: [slug("a"), slug("b")]
        }),
        "the first"
    );
    let elsewhere = [with("e", None, &[], &["o/r#13"])];
    assert_eq!(
        number_clashes(&elsewhere, &[of("e", 2, 13)]),
        [],
        "another repository's"
    );
    let others = [with("e", None, &[], &["#13"])];
    assert_eq!(
        number_clashes(&others, &[of("x", 2, 13)]),
        [],
        "another task's change"
    );
}

/// Every task holding an issue that is a pull request clashes with it; an
/// issue three tasks share is two pairs.
#[test]
fn an_issue_shared_by_many_clashes_for_each() {
    let shared = [
        with("a", Some(1), &[], &[]),
        with("b", Some(1), &[], &[]),
        with("c", Some(1), &[], &[]),
    ];
    assert_eq!(
        number_clashes(&shared, &[]),
        [
            BoardRuleError::IssueShared {
                issue: 1,
                tasks: [slug("a"), slug("b")]
            },
            BoardRuleError::IssueShared {
                issue: 1,
                tasks: [slug("b"), slug("c")]
            },
        ]
    );
    let also_pr = [
        with("a", Some(7), &[], &[]),
        with("b", Some(7), &[], &[]),
        with("d", None, &[7], &[]),
    ];
    let is_pr = |task: &str| BoardRuleError::IssueIsPr {
        issue: 7,
        task: slug(task),
        pr_holder: slug("d"),
    };
    assert_eq!(
        number_clashes(&also_pr, &[]),
        [
            BoardRuleError::IssueShared {
                issue: 7,
                tasks: [slug("a"), slug("b")]
            },
            is_pr("a"),
            is_pr("b"),
        ]
    );
}

/// Only a linked change's pull request is the task's own: one an
/// unlinked (abandoned) change named may be referred to.
#[test]
fn a_reference_to_an_abandoned_changes_pull_request_holds() {
    let mut fields = of("e", 2, 13).into_fields();
    fields.unlinked = Some((fields.linked_by.clone(), fields.updated.clone()));
    let abandoned = Change::new(fields).unwrap();
    let board = [with("e", None, &[], &["#13"])];
    assert_eq!(number_clashes(&board, &[abandoned]), []);
    assert_eq!(
        number_clashes(&board, &[of("e", 2, 13)]),
        [BoardRuleError::OwnPrReferenced {
            task: slug("e"),
            pr: 13,
            change: 2
        }]
    );
}
