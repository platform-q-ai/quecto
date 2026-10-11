use super::*;
use crate::domain::project_board::services::fixtures_tests::{
    at, change, commit, open_run, person, task,
};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::vocabulary::{RunOutcome, TaskStatus::*};

fn merged(prs: &[(u64, char)]) -> BTreeMap<u64, String> {
    prs.iter()
        .map(|(pr, digit)| (*pr, commit(*digit)))
        .collect()
}

fn unlinked(number: u32, pr: u64) -> Change {
    unlink(
        &task(InProgress),
        &change(number, Some(pr)),
        &person("Ada"),
        &at("10:00:00"),
    )
    .unwrap()
}

#[test]
fn the_holder_links_a_change_numbered_after_the_last() {
    let linked = link(
        &task(InProgress),
        &[change(1, Some(7))],
        "feat/second".into(),
        Some(8),
        &person("Ada"),
        &at("10:00:00"),
    )
    .unwrap();
    let fields = linked.fields();
    assert_eq!(
        (fields.number, fields.pr, &fields.linked_by),
        (2, Some(8), &person("Ada"))
    );
    assert_eq!(
        link(
            &task(InProgress),
            &[],
            "feat/x".into(),
            None,
            &person("Bob"),
            &at("10:00:00")
        ),
        Err(BoardRuleError::NotHolder)
    );
    assert_eq!(
        link(
            &task(Ready),
            &[],
            "feat/x".into(),
            None,
            &person("Ada"),
            &at("10:00:00")
        ),
        Err(BoardRuleError::NoClaim)
    );
}

/// A branch or pull request a linked change already names is refused; an
/// unlinked change's are free again.
#[test]
fn a_branch_or_pull_request_is_linked_once() {
    let ada = person("Ada");
    let now = at("10:00:00");
    let existing = [change(1, Some(7))];
    for (branch, pr) in [("feat/b1", None), ("feat/other", Some(7))] {
        assert_eq!(
            link(&task(InProgress), &existing, branch.into(), pr, &ada, &now),
            Err(BoardRuleError::ChangeAlreadyLinked(1)),
            "{branch} {pr:?}"
        );
    }
    let again = link(
        &task(InProgress),
        &[unlinked(1, 7)],
        "feat/b1".into(),
        Some(7),
        &ada,
        &now,
    );
    assert_eq!(again.map(|change| change.fields().number), Ok(2));
}

#[test]
fn a_change_is_unlinked_once_kept_and_never_after_it_merged() {
    let gone = unlinked(1, 7);
    assert!(!gone.is_linked());
    assert_eq!(
        gone.fields().unlinked,
        Some((person("Ada"), at("10:00:00")))
    );
    assert_eq!(
        unlink(&task(InProgress), &gone, &person("Ada"), &at("10:01:00")),
        Err(BoardRuleError::ChangeUnlinked(1))
    );
    let mut merged_fields = change(2, Some(8)).into_fields();
    merged_fields.merged = Some(commit('a'));
    let merged = Change::new(merged_fields).unwrap();
    assert_eq!(
        unlink(&task(InProgress), &merged, &person("Ada"), &at("10:00:00")),
        Err(BoardRuleError::ChangeMerged(2))
    );
    assert_eq!(
        unlink(
            &task(InProgress),
            &change(3, None),
            &person("Bob"),
            &at("10:00:00")
        ),
        Err(BoardRuleError::NotHolder)
    );
}

/// A task is done once every change it links has merged: each records
/// its merge commit, an unlinked one is left out, and a run going ends.
#[test]
fn a_task_is_done_when_its_linked_changes_merge() {
    let changes = [change(1, Some(7)), change(2, Some(8)), unlinked(3, 9)];
    let host = merged(&[(7, 'a'), (8, 'b')]);
    let done = complete(
        &task(Review),
        &changes,
        &host,
        &[open_run(1, 1)],
        &at("10:00:00"),
    )
    .unwrap();
    assert_eq!(done.completed.task.fields().status, Done);
    assert_eq!(
        done.completed.run.map(|run| run.fields().outcome),
        Some(Some(RunOutcome::Lost))
    );
    let commits: Vec<_> = done
        .changes
        .iter()
        .map(|change| (change.fields().number, change.fields().merged.clone()))
        .collect();
    assert_eq!(commits, [(1, Some(commit('a'))), (2, Some(commit('b')))]);
}

#[test]
fn a_task_is_not_done_while_a_linked_change_has_not_merged() {
    let now = at("10:00:00");
    let open = [change(1, Some(7)), change(2, Some(8)), change(3, None)];
    assert_eq!(
        complete(&task(Review), &open, &merged(&[(7, 'a')]), &[], &now).map(|done| done.changes),
        Err(BoardRuleError::ChangesUnmerged(vec![2, 3]))
    );
    assert_eq!(
        complete(
            &task(Review),
            &[unlinked(1, 7)],
            &merged(&[(7, 'a')]),
            &[],
            &now
        )
        .map(|done| done.changes),
        Err(BoardRuleError::NoChanges)
    );
    for status in [Draft, Done, Archived] {
        assert_eq!(
            complete(&task(status), &open, &merged(&[]), &[], &now).map(|done| done.changes),
            Err(BoardRuleError::NotAllowed {
                from: status,
                to: Done
            })
        );
    }
    let mut elsewhere = change(1, Some(7)).into_fields();
    elsewhere.task = Slug::parse("other").unwrap();
    assert_eq!(
        complete(
            &task(Review),
            &[Change::new(elsewhere).unwrap()],
            &merged(&[(7, 'a')]),
            &[],
            &now
        )
        .map(|done| done.changes),
        Err(BoardRuleError::NotThisTasks)
    );
}
