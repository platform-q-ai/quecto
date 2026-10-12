//! The board's issue numbers (#2481 S8): the checks a task cannot make
//! alone, which the importer and M4's write paths call with the board as
//! it will be. The host numbers issues and pull requests from one
//! sequence, so no two tasks share an issue, no task's issue is a pull
//! request anywhere on the board (linked the old way or by a change), and
//! no task names by `#n` a pull request one of its own changes links.
use std::collections::BTreeMap;

use super::super::entities::change::Change;
use super::super::entities::task::Task;
use super::super::value_objects::reference::Points;
use super::super::value_objects::rule_error::BoardRuleError;
use super::super::value_objects::slug::Slug;

/// Every clash among `tasks`' issues and references and the pull requests
/// of `tasks` and `changes`, each naming both sides, in the board's order
/// (by issue, then by task): none when the board holds. An import reports
/// them all.
pub fn number_clashes(tasks: &[Task], changes: &[Change]) -> Vec<BoardRuleError> {
    // Who links each pull request: a task the old way, or a change's task.
    let mut holders: BTreeMap<u64, &Slug> = BTreeMap::new();
    for task in tasks {
        for pr in &task.fields().prs {
            holders.entry(*pr).or_insert(&task.fields().id);
        }
    }
    for change in changes {
        let fields = change.fields();
        if let Some(pr) = fields.pr {
            holders.entry(pr).or_insert(&fields.task);
        }
    }
    let mut owners: BTreeMap<u64, Vec<&Slug>> = BTreeMap::new();
    for task in tasks {
        if let Some(issue) = task.fields().github_issue {
            owners
                .entry(issue.get())
                .or_default()
                .push(&task.fields().id);
        }
    }
    let mut clashes = vec![];
    for (issue, tasks) in &owners {
        for pair in tasks.windows(2) {
            clashes.push(BoardRuleError::IssueShared {
                issue: *issue,
                tasks: [pair[0].clone(), pair[1].clone()],
            });
        }
        if let Some(holder) = holders.get(issue) {
            clashes.push(BoardRuleError::IssueIsPr {
                issue: *issue,
                task: tasks[0].clone(),
                pr_holder: (*holder).clone(),
            });
        }
    }
    clashes.extend(own_prs_referenced(tasks, changes));
    clashes
}

/// The board's issue numbers hold: [`number_clashes`] finds none.
///
/// # Errors
/// The first clash: two tasks with one issue (`issue_shared`), an issue
/// that is a pull request on the board (`issue_is_pr`), or a reference to
/// a pull request of the task's own change (`own_pr_referenced`).
pub fn issue_numbers_hold(tasks: &[Task], changes: &[Change]) -> Result<(), BoardRuleError> {
    match number_clashes(tasks, changes).into_iter().next() {
        None => Ok(()),
        Some(clash) => Err(clash),
    }
}

/// Each `#n` reference of a task to a pull request one of its own changes
/// links (a task's own old-way pull requests are its own rule).
fn own_prs_referenced(tasks: &[Task], changes: &[Change]) -> Vec<BoardRuleError> {
    let referenced = |task: &Task, pr: u64| {
        task.fields().references.iter().any(
            |reference| matches!(reference.points(), Points::Own(number) if number.get() == pr),
        )
    };
    tasks
        .iter()
        .flat_map(|task| {
            changes
                .iter()
                .filter(move |change| change.fields().task == task.fields().id)
                .filter_map(move |change| {
                    let pr = change.fields().pr?;
                    referenced(task, pr).then(|| BoardRuleError::OwnPrReferenced {
                        task: task.fields().id.clone(),
                        pr,
                        change: change.fields().number,
                    })
                })
        })
        .collect()
}

#[cfg(test)]
#[path = "issue_numbers_tests.rs"]
mod tests;
