//! The board's issue numbers (#2481 S8): the checks a task cannot make
//! alone, which the importer and M4's write paths call with the board as
//! it will be. The host numbers issues and pull requests from one
//! sequence, so no two tasks share an issue and no task's issue is a pull
//! request anywhere on the board, linked the old way or by a change.
use std::collections::{BTreeMap, BTreeSet};

use super::super::entities::change::Change;
use super::super::entities::task::Task;
use super::super::value_objects::rule_error::BoardRuleError;
use super::super::value_objects::slug::Slug;

/// `tasks`' issues and the pull requests of `tasks` and `changes` agree:
/// each issue is one task's, and none is a pull request.
///
/// # Errors
/// Two tasks with one issue (`issue_shared`, naming the first); an issue
/// that is a pull request on the board (`issue_is_pr`, naming the task
/// whose issue it is).
pub fn issue_numbers_hold(tasks: &[Task], changes: &[Change]) -> Result<(), BoardRuleError> {
    let prs: BTreeSet<u64> = tasks
        .iter()
        .flat_map(|task| task.fields().prs.iter().copied())
        .chain(changes.iter().filter_map(|change| change.fields().pr))
        .collect();
    let mut seen: BTreeMap<u64, &Slug> = BTreeMap::new();
    for task in tasks {
        let fields = task.fields();
        let Some(issue) = fields.github_issue.map(|issue| issue.get()) else {
            continue;
        };
        match (seen.insert(issue, &fields.id), prs.contains(&issue)) {
            (None, false) => {}
            (Some(first), _) => {
                return Err(BoardRuleError::IssueShared {
                    issue,
                    task: first.clone(),
                });
            }
            (None, true) => {
                return Err(BoardRuleError::IssueIsPr {
                    issue,
                    task: fields.id.clone(),
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "issue_numbers_tests.rs"]
mod tests;
