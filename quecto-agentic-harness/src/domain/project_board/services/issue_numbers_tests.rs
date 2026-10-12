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
