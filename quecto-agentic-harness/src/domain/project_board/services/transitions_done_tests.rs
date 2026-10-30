//! #2705 T4 review round 1: a task done without a run waits as a claim
//! waits for the tasks it depends on and as an archive waits for its
//! children; and, by the owner's choice, one that links pull requests is
//! done so once every one has merged (changes alike).
use super::*;
use crate::domain::project_board::entities::change::Change;
use crate::domain::project_board::services::fixtures_tests::{
    at, change, person, task, without_prs,
};

const WHY: &str = "Fixed by hand: see the commit on main";

/// `task(status)` named `id`, unclaimed where it may be, depending on
/// `depends_on` and a child of `parent`.
fn other(id: &str, status: TaskStatus, parent: Option<&str>) -> Task {
    let mut fields = without_prs(task(status)).into_fields();
    fields.id = Slug::parse(id).unwrap();
    fields.parent = parent.map(|parent| Slug::parse(parent).unwrap());
    Task::new(fields).unwrap()
}

/// The fixture task in `status`, depending on `depends_on`.
fn depending(status: TaskStatus, depends_on: &[&str]) -> Task {
    let mut fields = without_prs(task(status)).into_fields();
    fields.depends_on = depends_on
        .iter()
        .map(|id| Slug::parse(id).unwrap())
        .collect();
    Task::new(fields).unwrap()
}

fn done(
    task: &Task,
    changes: &[Change],
    tasks: &[Task],
    merged: &[u64],
) -> Result<Task, BoardRuleError> {
    let merged: BTreeSet<u64> = merged.iter().copied().collect();
    let work = DoneWork {
        runs: &[],
        changes,
        tasks,
        merged: &merged,
    };
    done_without_run(task, WHY, &person("Ada"), work, &at("10:00:00"))
}

#[test]
fn a_task_whose_dependency_is_not_done_is_not_done_without_a_run() {
    let board = [other("t1", InProgress, None), other("t2", Done, None)];
    for status in [Ready, Claimed] {
        let held = depending(status, &["t1", "t2"]);
        assert_eq!(
            done(&held, &[], &board, &[]),
            Err(BoardRuleError::DependencyNotDone {
                undone: vec![Slug::parse("t1").unwrap()],
                missing: vec![],
            }),
            "{status:?}"
        );
        // One not on the board holds it too, as it holds a claim.
        assert!(
            matches!(
                done(&depending(status, &["gone"]), &[], &board, &[]),
                Err(BoardRuleError::DependencyNotDone { .. })
            ),
            "{status:?}"
        );
        let finished = depending(status, &["t2"]);
        assert_eq!(
            done(&finished, &[], &board, &[]).map(|task| task.fields().status),
            Ok(Done),
            "{status:?}: a done dependency lets it go"
        );
    }
}

#[test]
fn a_parent_with_a_child_at_work_is_not_done_without_a_run() {
    let parent = without_prs(task(Ready));
    for status in [Claimed, InProgress, Review, Blocked] {
        let board = [other("kid", status, Some("board-store"))];
        assert_eq!(
            done(&parent, &[], &board, &[]),
            Err(BoardRuleError::ChildrenActive(vec![
                Slug::parse("kid").unwrap()
            ])),
            "{status:?}"
        );
    }
    for status in [Draft, Ready, Done, Archived] {
        let board = [other("kid", status, Some("board-store"))];
        assert!(done(&parent, &[], &board, &[]).is_ok(), "{status:?}");
    }
}

#[test]
fn a_task_linking_pull_requests_is_done_without_a_run_once_every_one_has_merged() {
    let claimed = task(Claimed);
    assert_eq!(claimed.fields().prs, [2482, 2483]);
    assert_eq!(
        done(&claimed, &[], &[], &[2482]),
        Err(BoardRuleError::PrsOpen(vec![2483])),
        "names only the ones not merged"
    );
    let finished = done(&claimed, &[], &[], &[2482, 2483]).unwrap();
    let fields = finished.fields();
    assert_eq!(
        (fields.status, fields.resolution, fields.reason.as_deref()),
        (Done, Some(Resolution::DoneWithoutRun), Some(WHY))
    );
    assert_eq!(fields.prs, [2482, 2483], "it keeps what delivered it");
}

#[test]
fn a_merged_change_lets_it_go_and_an_open_one_holds_it() {
    let held = without_prs(task(Claimed));
    let open = change(1, Some(9001));
    assert_eq!(
        done(&held, std::slice::from_ref(&open), &[], &[]),
        Err(BoardRuleError::ChangesUnmerged(vec![1]))
    );
    let mut fields = open.into_fields();
    fields.merged = Some(crate::domain::project_board::services::fixtures_tests::commit('a'));
    let merged = Change::new(fields).unwrap();
    assert!(done(&held, &[merged], &[], &[]).is_ok());
}

#[test]
fn done_ticks_review_to_done_settles_outcomes_preserving_sections() {
    use crate::domain::project_board::entities::task_parts::DescriptionForm;
    let description = "## Goal\nShip.\n\n## Why\nKeep this.\n\n## Done when\n- [ ] First\n- [x] Second\n- [ ] Third";
    let mut fields = without_prs(task(Review)).into_fields();
    fields.description = description.into();
    fields.description_form = DescriptionForm::Sectioned;
    let reviewed = Task::new(fields).unwrap();
    let finished = move_task(
        &reviewed,
        Done,
        &person("Ada"),
        (&[], &[]),
        &[],
        &at("10:00:00"),
    )
    .unwrap();
    assert_eq!(finished.fields().status, Done);
    assert_eq!(
        finished.fields().description,
        description.replace("- [ ]", "- [x]")
    );
}
