use super::*;
use crate::domain::project_board::services::fixtures_tests::{
    at, branch, change, commit, open_run, person, repo, task, without_prs,
};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::vocabulary::{
    RunOutcome, TaskStatus, TaskStatus::*,
};

const OURS: &str = "platform-q-ai/quecto";

/// The fixture task in `status`, linking no pull request the old way.
fn of(status: TaskStatus) -> Task {
    without_prs(task(status))
}

fn now() -> Moment {
    Moment {
        now: at("10:00:00"),
        latest: None,
    }
}

/// The board's repository and `name`, with pull request `pr`.
fn head(name: &str, pr: Option<u64>) -> Head {
    Head {
        repo: repo(OURS),
        branch: branch(name),
        pr,
    }
}

/// Each `(pr, branch, state)` as the host reads it, from our repository.
fn host(prs: &[(u64, &str, PrState)]) -> HostPrs {
    prs.iter()
        .map(|(pr, name, state)| {
            let read = HostPr {
                head_repo: repo(OURS),
                head_ref: branch(name),
                state: state.clone(),
            };
            (*pr, read)
        })
        .collect()
}

fn merged(digit: char) -> PrState {
    PrState::Merged(commit(digit))
}

fn unlinked(number: u32, pr: u64) -> Change {
    let own = change(number, Some(pr));
    let read = host(&[(pr, &format!("feat/b{number}"), PrState::Open)]);
    unlink(
        &of(InProgress),
        std::slice::from_ref(&own),
        number,
        &read,
        &person("Ada"),
        &now(),
    )
    .unwrap()
}

fn recorded(number: u32, pr: Option<u64>, digit: char) -> Change {
    let mut fields = change(number, pr).into_fields();
    fields.merged = Some(commit(digit));
    Change::new(fields).unwrap()
}

/// Another task's change on branch `feat/o<number>`.
fn elsewhere(number: u32, pr: Option<u64>) -> Change {
    let mut fields = change(number, pr).into_fields();
    fields.task = Slug::parse("other").unwrap();
    fields.branch = branch(&format!("feat/o{number}"));
    Change::new(fields).unwrap()
}

fn stamped(change: Change, updated: &str) -> Change {
    let mut fields = change.into_fields();
    fields.updated = at(updated);
    Change::new(fields).unwrap()
}

#[test]
fn the_holder_links_a_change_numbered_after_the_tasks_last() {
    let board = [change(1, Some(7)), elsewhere(5, Some(40))];
    let read = host(&[(8, "feat/second", PrState::Open)]);
    let linked = link(
        &of(InProgress),
        &board,
        head("feat/second", Some(8)),
        &read,
        &person("Ada"),
        &now(),
    )
    .unwrap();
    let fields = linked.fields();
    assert_eq!(
        (fields.number, fields.pr, &fields.linked_by, &fields.updated),
        (2, Some(8), &person("Ada"), &at("10:00:00"))
    );
    assert_eq!(linked.head(), (&repo(OURS), &branch("feat/second")));
    let none = HostPrs::new();
    assert_eq!(
        link(
            &of(InProgress),
            &[],
            head("feat/x", None),
            &none,
            &person("Bob"),
            &now()
        ),
        Err(BoardRuleError::NotHolder)
    );
    assert_eq!(
        link(
            &of(Ready),
            &[],
            head("feat/x", None),
            &none,
            &person("Ada"),
            &now()
        ),
        Err(BoardRuleError::NoClaim)
    );
    let mut full = change(1, None).into_fields();
    full.number = MAX_CHANGE_NUMBER;
    let full = Change::new(full).unwrap();
    let past = link(
        &of(InProgress),
        &[full],
        head("feat/x", None),
        &none,
        &person("Ada"),
        &now(),
    );
    assert!(
        matches!(&past, Err(BoardRuleError::Invalid(error)) if error.field == "number"),
        "{past:?}"
    );
}

/// A pull request is linked to one task for good; a branch of a
/// repository to one open change, so it is free again once that change
/// merges or is unlinked; the same branch name in a fork is another
/// branch; and two changes may both lack a pull request.
#[test]
fn a_pull_request_or_open_branch_is_linked_once_on_the_board() {
    let ada = person("Ada");
    let board = [
        change(1, Some(7)),
        elsewhere(1, Some(40)),
        elsewhere(2, None),
    ];
    let read = host(&[
        (7, "feat/new", PrState::Open),
        (40, "feat/new", PrState::Open),
    ]);
    let other = Slug::parse("other").unwrap();
    for (name, pr, task, change) in [
        ("feat/b1", None, of(InProgress).fields().id.clone(), 1),
        ("feat/new", Some(7), of(InProgress).fields().id.clone(), 1),
        ("feat/new", Some(40), other.clone(), 1),
        ("feat/o2", None, other, 2),
    ] {
        assert_eq!(
            link(&of(InProgress), &board, head(name, pr), &read, &ada, &now()),
            Err(BoardRuleError::ChangeAlreadyLinked { task, change }),
            "{name} {pr:?}"
        );
    }
    let fork = Head {
        repo: repo("someone/quecto"),
        branch: branch("feat/b1"),
        pr: None,
    };
    let forked = link(&of(InProgress), &board, fork, &read, &ada, &now());
    assert_eq!(
        forked.map(|change| change.fields().number),
        Ok(2),
        "a fork's branch of that name"
    );
    let read = host(&[(7, "feat/b1", PrState::Open)]);
    let again = link(
        &of(InProgress),
        &[unlinked(1, 7)],
        head("feat/b1", Some(7)),
        &read,
        &ada,
        &now(),
    );
    assert_eq!(
        again.map(|change| change.fields().number),
        Ok(2),
        "unlinked: both free"
    );
    let done = [recorded(1, Some(7), 'a')];
    let reused = link(
        &of(InProgress),
        &done,
        head("feat/b1", None),
        &read,
        &ada,
        &now(),
    );
    assert_eq!(
        reused.map(|change| change.fields().number),
        Ok(2),
        "merged: the branch is free"
    );
    assert_eq!(
        link(
            &of(InProgress),
            &done,
            head("feat/b1", Some(7)),
            &read,
            &ada,
            &now()
        ),
        Err(BoardRuleError::ChangeAlreadyLinked {
            task: of(InProgress).fields().id.clone(),
            change: 1
        }),
        "merged: the pull request is linked for good, whatever the host says"
    );
    let bare = link(
        &of(InProgress),
        &[change(1, None)],
        head("feat/two", None),
        &read,
        &ada,
        &now(),
    );
    assert_eq!(bare.map(|change| change.fields().number), Ok(2));
}

/// The pull request linked is the host's open one from the change's head,
/// and not one the task links the old way.
#[test]
fn a_pull_request_is_linked_as_the_host_reads_it() {
    let ada = person("Ada");
    let linking =
        |task: &Task, read: &HostPrs| link(task, &[], head("feat/x", Some(7)), read, &ada, &now());
    assert_eq!(
        linking(&of(InProgress), &HostPrs::new()),
        Err(BoardRuleError::PrUnread(7))
    );
    for state in [PrState::Closed, merged('a')] {
        assert_eq!(
            linking(&of(InProgress), &host(&[(7, "feat/x", state)])),
            Err(BoardRuleError::PrNotOpen(7))
        );
    }
    assert_eq!(
        linking(&of(InProgress), &host(&[(7, "feat/y", PrState::Open)])),
        Err(BoardRuleError::PrHeadMismatch(7))
    );
    let mut fork = host(&[(7, "feat/x", PrState::Open)]);
    fork.get_mut(&7).unwrap().head_repo = repo("someone/quecto");
    assert_eq!(
        linking(&of(InProgress), &fork),
        Err(BoardRuleError::PrHeadMismatch(7))
    );
    let old_way = link(
        &task(InProgress),
        &[],
        head("feat/x", Some(2482)),
        &host(&[(2482, "feat/x", PrState::Open)]),
        &ada,
        &now(),
    );
    assert_eq!(old_way, Err(BoardRuleError::PrAlreadyLinked(2482)));
    assert!(linking(&of(InProgress), &host(&[(7, "feat/x", PrState::Open)])).is_ok());
}

#[test]
fn a_change_is_unlinked_once_kept_after_its_last_change_and_never_after_it_merged() {
    let ada = person("Ada");
    let open = host(&[
        (7, "feat/b1", PrState::Open),
        (8, "feat/b2", PrState::Closed),
    ]);
    let own = [change(1, Some(7)), change(2, Some(8)), change(3, None)];
    let early = Moment {
        now: at("08:00:00"),
        latest: None,
    };
    let gone = unlink(&of(InProgress), &own, 1, &open, &ada, &early).unwrap();
    assert!(!gone.is_linked());
    assert_eq!(gone.fields().unlinked, Some((ada.clone(), at("09:00:01"))));
    assert_eq!(
        gone.fields().updated,
        at("09:00:01"),
        "a clock behind stamps forward"
    );
    assert!(
        unlink(&of(InProgress), &own, 2, &open, &ada, &now()).is_ok(),
        "closed unmerged"
    );
    assert!(
        unlink(&of(InProgress), &own, 3, &HostPrs::new(), &ada, &now()).is_ok(),
        "no pull request"
    );
    assert_eq!(
        unlink(&of(InProgress), &[gone], 1, &open, &ada, &now()),
        Err(BoardRuleError::ChangeUnlinked(1))
    );
    assert_eq!(
        unlink(&of(InProgress), &own, 4, &open, &ada, &now()),
        Err(BoardRuleError::NoSuchChange(4))
    );
    assert_eq!(
        unlink(
            &of(InProgress),
            &[recorded(1, Some(7), 'a')],
            1,
            &open,
            &ada,
            &now()
        ),
        Err(BoardRuleError::ChangeMerged(1))
    );
    assert_eq!(
        unlink(
            &of(InProgress),
            &own,
            1,
            &host(&[(7, "feat/b1", merged('a'))]),
            &ada,
            &now()
        ),
        Err(BoardRuleError::ChangeMerged(1)),
        "merged on the host"
    );
    assert_eq!(
        unlink(&of(InProgress), &own, 1, &HostPrs::new(), &ada, &now()),
        Err(BoardRuleError::PrUnread(7))
    );
    assert_eq!(
        unlink(&of(InProgress), &own, 3, &open, &person("Bob"), &now()),
        Err(BoardRuleError::NotHolder)
    );
}

/// A task is done once every change it links has merged: each records
/// its merge commit, an unlinked one is left out, and a run going ends.
#[test]
fn a_task_is_done_when_its_linked_changes_merge() {
    let changes = [change(1, Some(7)), change(2, Some(8)), unlinked(3, 9)];
    let read = host(&[(7, "feat/b1", merged('a')), (8, "feat/b2", merged('b'))]);
    let done = complete(&of(Review), &changes, &read, &[open_run(1, 1)], &now()).unwrap();
    assert_eq!(done.completed.task.fields().status, Done);
    assert_eq!(
        done.completed.run.map(|run| run.fields().outcome),
        Some(Some(RunOutcome::Lost))
    );
    let recorded: Vec<_> = done
        .changes
        .iter()
        .map(|change| {
            (
                change.fields().number,
                change.fields().merged.clone(),
                change.fields().updated.clone(),
            )
        })
        .collect();
    assert_eq!(
        recorded,
        [
            (1, Some(commit('a')), at("10:00:00")),
            (2, Some(commit('b')), at("10:00:00"))
        ]
    );
}

/// A change stamped after the task's completion stamp records its merge
/// a second past its own last change; one a clock ran ahead stamped is
/// refused once an outside time bounds the clock.
#[test]
fn a_change_stamped_later_keeps_its_order_unless_it_ran_ahead() {
    let late = [stamped(change(1, Some(7)), "10:30:00")];
    let read = host(&[(7, "feat/b1", merged('a'))]);
    let done = complete(&of(Review), &late, &read, &[], &now()).unwrap();
    assert_eq!(done.completed.task.fields().updated, at("10:00:00"));
    assert_eq!(done.changes[0].fields().updated, at("10:30:01"));
    let bounded = Moment {
        now: at("10:00:00"),
        latest: Some(at("10:00:00")),
    };
    let ahead = BoardRuleError::ChangeAhead {
        change: 1,
        updated: at("10:30:00"),
    };
    assert_eq!(
        complete(&of(Review), &late, &read, &[], &bounded).map(|done| done.changes),
        Err(ahead.clone())
    );
    let open = host(&[(7, "feat/b1", PrState::Open)]);
    assert_eq!(
        unlink(&of(InProgress), &late, 1, &open, &person("Ada"), &bounded),
        Err(ahead.clone())
    );
    let linking = link(
        &of(InProgress),
        &late,
        head("feat/x", None),
        &open,
        &person("Ada"),
        &bounded,
    );
    assert_eq!(linking, Err(ahead));
    let slack = Moment {
        now: at("10:00:00"),
        latest: Some(at("10:29:30")),
    };
    assert!(
        complete(&of(Review), &late, &read, &[], &slack).is_ok(),
        "within the stamp's slack"
    );
}

#[test]
fn a_task_is_not_done_while_something_it_links_has_not_merged() {
    let open = [change(1, Some(7)), change(2, Some(8)), change(3, None)];
    let refused = |task: &Task, changes: &[Change], read: &HostPrs| {
        complete(task, changes, read, &[], &now()).map(|done| done.changes)
    };
    let both = host(&[(7, "feat/b1", merged('a')), (8, "feat/b2", PrState::Closed)]);
    assert_eq!(
        refused(&of(Review), &open, &both),
        Err(BoardRuleError::ChangesUnmerged(vec![2, 3]))
    );
    let one = host(&[(7, "feat/b1", merged('a'))]);
    assert_eq!(
        refused(&of(Review), &[unlinked(1, 7)], &one),
        Err(BoardRuleError::NoChanges)
    );
    assert_eq!(
        refused(
            &of(Review),
            &[change(1, Some(7))],
            &host(&[(7, "feat/b2", merged('a'))])
        ),
        Err(BoardRuleError::ChangeHeadMismatch(1)),
        "merged from another branch"
    );
    let mut fork = one.clone();
    fork.get_mut(&7).unwrap().head_repo = repo("someone/quecto");
    assert_eq!(
        refused(&of(Review), &[change(1, Some(7))], &fork),
        Err(BoardRuleError::ChangeHeadMismatch(1)),
        "merged from a fork's branch of that name"
    );
    assert_eq!(
        refused(&of(Review), &[recorded(1, Some(7), 'b')], &one),
        Err(BoardRuleError::ChangeMerged(1)),
        "a recorded merge is never overwritten"
    );
    for status in [Draft, Done, Archived] {
        assert_eq!(
            refused(&of(status), &open, &one),
            Err(BoardRuleError::NotAllowed {
                from: status,
                to: Done
            })
        );
    }
    assert_eq!(
        refused(&of(Review), &[elsewhere(1, Some(7))], &one),
        Err(BoardRuleError::NotThisTasks)
    );
    let old_way = host(&[(7, "feat/b1", merged('a')), (2482, "feat/x", merged('c'))]);
    assert_eq!(
        refused(&task(Review), &[change(1, Some(7))], &old_way),
        Err(BoardRuleError::PrsUnmerged(vec![2483])),
        "the pull requests linked the old way merge too"
    );
    let mut all = old_way.clone();
    all.insert(
        2483,
        HostPr {
            head_repo: repo(OURS),
            head_ref: branch("feat/y"),
            state: merged('d'),
        },
    );
    assert!(refused(&task(Review), &[change(1, Some(7))], &all).is_ok());
}

/// The task's changes are judged as the board holds them: a number twice,
/// a pull request linked twice, or a branch open twice is refused by every
/// rule; a branch unlinked or merged and linked again is not.
#[test]
fn changes_that_contradict_each_other_are_refused() {
    let ada = person("Ada");
    let read = host(&[(7, "feat/b1", PrState::Open), (8, "feat/b1", PrState::Open)]);
    let mut same_branch = change(2, Some(8)).into_fields();
    same_branch.branch = branch("feat/b1");
    let mut same_pr = change(2, Some(7)).into_fields();
    same_pr.branch = branch("feat/b2");
    for (contradiction, why) in [
        ([change(1, Some(7)), change(1, Some(8))], "number"),
        (
            [change(1, Some(7)), Change::new(same_pr).unwrap()],
            "pull request",
        ),
        (
            [
                change(1, Some(7)),
                Change::new(same_branch.clone()).unwrap(),
            ],
            "branch",
        ),
    ] {
        let inconsistent = |result: Result<Change, BoardRuleError>| matches!(result, Err(BoardRuleError::ChangesInconsistent(text)) if text.contains(why));
        assert!(
            inconsistent(link(
                &of(InProgress),
                &contradiction,
                head("feat/z", None),
                &read,
                &ada,
                &now()
            )),
            "{why}"
        );
        assert!(
            inconsistent(unlink(
                &of(InProgress),
                &contradiction,
                1,
                &read,
                &ada,
                &now()
            )),
            "{why}"
        );
        let completing = complete(&of(Review), &contradiction, &read, &[], &now());
        assert!(
            matches!(completing, Err(BoardRuleError::ChangesInconsistent(_))),
            "{why}"
        );
    }
    let relinked = [unlinked(1, 7), Change::new(same_branch).unwrap()];
    assert!(
        link(
            &of(InProgress),
            &relinked,
            head("feat/z", None),
            &read,
            &ada,
            &now()
        )
        .is_ok()
    );
    assert!(unlink(&of(InProgress), &relinked, 2, &read, &ada, &now()).is_ok());
    let merged_both = host(&[(8, "feat/b1", merged('a'))]);
    assert_eq!(
        complete(&of(Review), &relinked, &merged_both, &[], &now()).map(|done| done.changes.len()),
        Ok(1)
    );
}
