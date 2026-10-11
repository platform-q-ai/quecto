use super::*;
use crate::domain::project_board::entities::change::Change;
use crate::domain::project_board::services::fixtures_tests::{change, task, without_prs};
use crate::domain::project_board::value_objects::reference::IssueNumber;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;

/// Task `id`, with issue `issue` and the old-way pull requests `prs`.
fn with(id: &str, issue: Option<u64>, prs: &[u64]) -> Task {
    let mut fields = without_prs(task(TaskStatus::Ready)).into_fields();
    fields.id = Slug::parse(id).unwrap();
    fields.github_issue = issue.map(|issue| IssueNumber::new(issue).unwrap());
    fields.prs = prs.to_vec();
    Task::new(fields).unwrap()
}

fn of(id: &str, change: Change) -> Change {
    let mut fields = change.into_fields();
    fields.task = Slug::parse(id).unwrap();
    Change::new(fields).unwrap()
}

#[test]
fn each_issue_is_one_tasks_and_no_pull_request() {
    let board = [
        with("a", Some(1), &[10]),
        with("b", Some(2), &[]),
        with("c", None, &[11]),
    ];
    assert_eq!(
        issue_numbers_hold(&board, &[of("c", change(1, Some(12)))]),
        Ok(())
    );
    let shared = [with("a", Some(1), &[]), with("b", Some(1), &[])];
    assert_eq!(
        issue_numbers_hold(&shared, &[]),
        Err(BoardRuleError::IssueShared {
            issue: 1,
            task: Slug::parse("a").unwrap()
        })
    );
    let old_way = [with("a", Some(11), &[]), with("c", None, &[11])];
    let is_pr = Err(BoardRuleError::IssueIsPr {
        issue: 11,
        task: Slug::parse("a").unwrap(),
    });
    assert_eq!(issue_numbers_hold(&old_way, &[]), is_pr);
    let by_change = [with("a", Some(11), &[])];
    assert_eq!(
        issue_numbers_hold(&by_change, &[of("b", change(1, Some(11)))]),
        is_pr
    );
    assert_eq!(issue_numbers_hold(&[], &[]), Ok(()));
}
