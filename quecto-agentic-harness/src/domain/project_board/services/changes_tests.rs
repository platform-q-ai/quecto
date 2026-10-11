use super::*;
use crate::domain::project_board::services::fixtures_tests::{
    at, branch, change, commit, open_run, person, task,
};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::vocabulary::{RunOutcome, TaskStatus::*};

/// Each `(pr, merge commit digit, change number)` as the host reads it,
/// merged from that change's branch.
fn merged(prs: &[(u64, char, u32)]) -> BTreeMap<u64, MergedPr> {
    prs.iter()
        .map(|(pr, digit, number)| {
            let read = MergedPr {
                merge_commit: commit(*digit),
                head_ref: branch(&format!("feat/b{number}")),
            };
            (*pr, read)
        })
        .collect()
}

fn unlinked(number: u32, pr: u64) -> Change {
    let own = change(number, Some(pr));
    unlink(
        &task(InProgress),
        std::slice::from_ref(&own),
        &own,
        &person("Ada"),
        &at("10:00:00"),
    )
    .unwrap()
}

fn elsewhere(number: u32, pr: Option<u64>) -> Change {
    let mut fields = change(number, pr).into_fields();
    fields.task = Slug::parse("other").unwrap();
    Change::new(fields).unwrap()
}

#[test]
fn the_holder_links_a_change_numbered_after_the_tasks_last() {
    let board = [change(1, Some(7)), elsewhere(5, Some(40))];
    let linked = link(
        &task(InProgress),
        &board,
        branch("feat/second"),
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
            branch("feat/x"),
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
            branch("feat/x"),
            None,
            &person("Ada"),
            &at("10:00:00")
        ),
        Err(BoardRuleError::NoClaim)
    );
    let mut full = change(1, None).into_fields();
    full.number = MAX_CHANGE_NUMBER;
    let full = Change::new(full).unwrap();
    let past = link(
        &task(InProgress),
        &[full],
        branch("feat/x"),
        None,
        &person("Ada"),
        &at("10:00:00"),
    );
    assert!(
        matches!(&past, Err(BoardRuleError::Invalid(error)) if error.field == "number"),
        "{past:?}"
    );
}

/// A branch or pull request is linked to one task on the whole board; an
/// unlinked change's are free again, and two changes may both lack a pull
/// request.
#[test]
fn a_branch_or_pull_request_is_linked_once_on_the_board() {
    let ada = person("Ada");
    let now = at("10:00:00");
    let board = [change(1, Some(7)), elsewhere(1, Some(40))];
    for (name, pr) in [
        ("feat/b1", None),
        ("feat/other", Some(7)),
        ("feat/x", Some(40)),
    ] {
        assert_eq!(
            link(&task(InProgress), &board, branch(name), pr, &ada, &now),
            Err(BoardRuleError::ChangeAlreadyLinked(1)),
            "{name} {pr:?}"
        );
    }
    let again = link(
        &task(InProgress),
        &[unlinked(1, 7)],
        branch("feat/b1"),
        Some(7),
        &ada,
        &now,
    );
    assert_eq!(again.map(|change| change.fields().number), Ok(2));
    let bare = link(
        &task(InProgress),
        &[change(1, None)],
        branch("feat/two"),
        None,
        &ada,
        &now,
    );
    assert_eq!(bare.map(|change| change.fields().number), Ok(2));
}

#[test]
fn a_change_is_unlinked_once_kept_after_its_last_change_and_never_after_it_merged() {
    let own = change(1, Some(7));
    let gone = unlink(
        &task(InProgress),
        std::slice::from_ref(&own),
        &own,
        &person("Ada"),
        &at("08:00:00"),
    )
    .unwrap();
    assert!(!gone.is_linked());
    assert_eq!(
        gone.fields().unlinked,
        Some((person("Ada"), at("09:00:01")))
    );
    assert_eq!(
        gone.fields().updated,
        at("09:00:01"),
        "a clock behind stamps forward"
    );
    assert_eq!(
        unlink(
            &task(InProgress),
            std::slice::from_ref(&gone),
            &gone,
            &person("Ada"),
            &at("10:01:00")
        ),
        Err(BoardRuleError::ChangeUnlinked(1))
    );
    let mut merged_fields = change(2, Some(8)).into_fields();
    merged_fields.merged = Some(commit('a'));
    let merged = Change::new(merged_fields).unwrap();
    assert_eq!(
        unlink(
            &task(InProgress),
            std::slice::from_ref(&merged),
            &merged,
            &person("Ada"),
            &at("10:00:00")
        ),
        Err(BoardRuleError::ChangeMerged(2))
    );
    assert_eq!(
        unlink(
            &task(InProgress),
            &[],
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
    let host = merged(&[(7, 'a', 1), (8, 'b', 2)]);
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
    let recorded: Vec<_> = done
        .changes
        .iter()
        .map(|change| {
            let fields = change.fields();
            (fields.number, fields.merged.clone(), fields.updated.clone())
        })
        .collect();
    assert_eq!(
        recorded,
        [
            (1, Some(commit('a')), at("10:00:00")),
            (2, Some(commit('b')), at("10:00:00")),
        ]
    );
}

#[test]
fn a_task_is_not_done_while_a_linked_change_has_not_merged() {
    let now = at("10:00:00");
    let open = [change(1, Some(7)), change(2, Some(8)), change(3, None)];
    let refused = |changes: &[Change], host: &BTreeMap<u64, MergedPr>| {
        complete(&task(Review), changes, host, &[], &now).map(|done| done.changes)
    };
    assert_eq!(
        refused(&open, &merged(&[(7, 'a', 1)])),
        Err(BoardRuleError::ChangesUnmerged(vec![2, 3]))
    );
    assert_eq!(
        refused(&[unlinked(1, 7)], &merged(&[(7, 'a', 1)])),
        Err(BoardRuleError::NoChanges)
    );
    assert_eq!(
        refused(&[change(1, Some(7))], &merged(&[(7, 'a', 2)])),
        Err(BoardRuleError::ChangeHeadMismatch(1)),
        "merged from another branch"
    );
    let mut recorded = change(1, Some(7)).into_fields();
    recorded.merged = Some(commit('b'));
    assert_eq!(
        refused(&[Change::new(recorded).unwrap()], &merged(&[(7, 'a', 1)])),
        Err(BoardRuleError::ChangeMerged(1)),
        "a recorded merge is never overwritten"
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
    assert_eq!(
        refused(&[elsewhere(1, Some(7))], &merged(&[(7, 'a', 1)])),
        Err(BoardRuleError::NotThisTasks)
    );
}

/// The task's changes are judged as the board holds them: a number twice,
/// or a branch or pull request linked twice, is refused by every rule.
#[test]
fn changes_that_contradict_each_other_are_refused() {
    let now = at("10:00:00");
    let ada = person("Ada");
    let mut same_branch = change(2, Some(8)).into_fields();
    same_branch.branch = branch("feat/b1");
    let mut same_pr = change(2, Some(7)).into_fields();
    same_pr.branch = branch("feat/b2");
    for (contradiction, why) in [
        ([change(1, Some(7)), change(1, Some(8))], "number"),
        (
            [change(1, Some(7)), Change::new(same_branch).unwrap()],
            "branch",
        ),
        (
            [change(1, Some(7)), Change::new(same_pr).unwrap()],
            "pull request",
        ),
    ] {
        let inconsistent = |result: Result<Change, BoardRuleError>| matches!(result, Err(BoardRuleError::ChangesInconsistent(text)) if text.contains(why));
        assert!(
            inconsistent(link(
                &task(InProgress),
                &contradiction,
                branch("feat/z"),
                None,
                &ada,
                &now
            )),
            "{why}"
        );
        assert!(
            inconsistent(unlink(
                &task(InProgress),
                &contradiction,
                &contradiction[0],
                &ada,
                &now
            )),
            "{why}"
        );
        assert!(
            matches!(
                complete(&task(Review), &contradiction, &merged(&[]), &[], &now),
                Err(BoardRuleError::ChangesInconsistent(_))
            ),
            "{why}"
        );
    }
}
