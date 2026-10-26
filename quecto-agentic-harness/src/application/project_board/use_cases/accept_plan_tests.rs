//! #2481 M4c: an acceptance names a draft as the task's plan and
//! supersedes the one accepted before, in one change of three files.
use super::AcceptPlan;
use crate::application::project_board::dto::ReadPin;
use crate::application::project_board::dto::{PlanAcceptance, ProjectBoardError};
use crate::application::project_board::ports::{ArtifactKey, BoardArtifact, ProjectBoardStore};
use crate::application::project_board::telemetry::OpFacts;
use crate::application::project_board::use_cases::fixtures_tests::{
    SetClock, at, board, fields, person, plan_of, slug,
};
use crate::domain::project_board::entities::claim::Claim;
use crate::domain::project_board::entities::plan::{Plan, PlanEntry};
use crate::domain::project_board::entities::run::Run;
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::vocabulary::{PlanState, TaskStatus};
use crate::infrastructure::test_support::board_artifacts::run;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use std::sync::Arc;

/// A board holding task `a` (ready, written at 09:00, naming `accepted`)
/// and `plans` and `runs` of it.
fn seeded(
    fields_of: impl FnOnce(TaskFields) -> TaskFields,
    plans: Vec<Plan>,
    runs: Vec<Run>,
) -> MemoryBoardStore {
    let store = board();
    let head = store.read().unwrap().head;
    let task = Task::new(fields_of(TaskFields {
        created: at("09:00:00"),
        updated: at("09:00:00"),
        ..fields("a")
    }))
    .unwrap();
    let artifacts: Vec<BoardArtifact> = std::iter::once(BoardArtifact::Task(task))
        .chain(plans.into_iter().map(BoardArtifact::Plan))
        .chain(runs.into_iter().map(BoardArtifact::Run))
        .collect();
    store.write(&head, &artifacts, "seed").unwrap();
    store
}

/// Task `a` naming version `version` as accepted.
fn naming(version: Option<u32>) -> impl FnOnce(TaskFields) -> TaskFields {
    move |fields| TaskFields {
        accepted_plan: version,
        ..fields
    }
}

fn accept_as(store: &MemoryBoardStore, writer: &str) -> AcceptPlan {
    let client = store.client_as(person(writer));
    AcceptPlan::new(Arc::new(client), SetClock::at("10:00:00")).unwrap()
}

/// Draft `version` of task `a`, as read at 09:00, with the task seen
/// naming `accepted`.
fn of(version: u32, accepted: Option<u32>) -> PlanAcceptance {
    PlanAcceptance {
        task: slug("a"),
        version,
        expected: ReadPin::Updated(at("09:00:00")),
        expected_accepted: accepted,
    }
}

fn states(store: &MemoryBoardStore) -> Vec<(u32, PlanState)> {
    let board = store.read().unwrap();
    board
        .plans
        .iter()
        .map(|plan| (plan.fields().version, plan.fields().state))
        .collect()
}

fn rule(rule: BoardRuleError) -> Result<(), ProjectBoardError> {
    Err(ProjectBoardError::Rule(rule))
}

#[test]
fn accepting_names_the_draft_and_supersedes_the_plan_accepted_before() {
    let store = seeded(
        naming(Some(1)),
        vec![
            plan_of("a", 1, PlanState::Accepted),
            plan_of("a", 2, PlanState::Draft),
        ],
        vec![],
    );
    let mut facts = OpFacts::default();
    let accepted = accept_as(&store, "Ada")
        .execute_noted(&of(2, Some(1)), Some(&mut facts))
        .unwrap();
    assert_eq!(accepted.task.fields().accepted_plan, Some(2));
    assert_eq!(accepted.task.fields().updated, at("10:00:00"));
    let plan = accepted.plan.fields();
    assert_eq!(
        (plan.version, plan.state, &plan.accepted),
        (
            2,
            PlanState::Accepted,
            &Some((person("Ada"), at("10:00:00")))
        )
    );
    let superseded = accepted.superseded.as_ref().unwrap().fields();
    assert_eq!(
        (superseded.version, superseded.state),
        (1, PlanState::Superseded)
    );
    assert_eq!(
        states(&store),
        [(1, PlanState::Superseded), (2, PlanState::Accepted)]
    );
    let board = store.read().unwrap();
    assert_eq!(board.tasks, vec![accepted.task.clone()]);
    assert_eq!(
        store.message_at(&board.head).as_deref(),
        Some("board: accept plan 2 of a")
    );
    let want = OpFacts {
        attempts: Some(1),
        lost: Some(0),
        plan_version: Some(2),
        plan_state_from: Some(PlanState::Draft),
        plan_state_to: Some(PlanState::Accepted),
        // The commit the write made (#2663).
        head: Some(crate::application::project_board::ports::BoardHead(
            "r3".into(),
        )),
        ..OpFacts::default()
    };
    assert_eq!(facts, want);
}

/// The first plan accepted supersedes none; an older draft accepted over
/// a newer accepted version is a rollback; a superseded plan is never
/// accepted again.
#[test]
fn a_first_acceptance_supersedes_none_and_an_older_draft_rolls_back() {
    let first = seeded(
        naming(None),
        vec![plan_of("a", 1, PlanState::Draft)],
        vec![],
    );
    let accepted = accept_as(&first, "Ada").execute(&of(1, None)).unwrap();
    assert_eq!(accepted.superseded, None);
    let store = seeded(
        naming(Some(2)),
        vec![
            plan_of("a", 1, PlanState::Draft),
            plan_of("a", 2, PlanState::Accepted),
        ],
        vec![],
    );
    let rolled_back = accept_as(&store, "Ada").execute(&of(1, Some(2))).unwrap();
    assert_eq!(rolled_back.task.fields().accepted_plan, Some(1));
    assert_eq!(
        states(&store),
        [(1, PlanState::Accepted), (2, PlanState::Superseded)]
    );
    let writes = store.writes();
    assert_eq!(
        accept_as(&store, "Ada")
            .execute(&of(2, Some(1)))
            .map(|_| ()),
        rule(BoardRuleError::PlanNotDraft(PlanState::Superseded))
    );
    assert_eq!(store.writes(), writes);
}

#[test]
fn nothing_is_accepted_while_a_run_is_open() {
    let store = seeded(
        naming(Some(1)),
        vec![
            plan_of("a", 1, PlanState::Accepted),
            plan_of("a", 2, PlanState::Draft),
        ],
        vec![run("a", 1)],
    );
    let mut facts = OpFacts::default();
    assert_eq!(
        accept_as(&store, "Ada")
            .execute_noted(&of(2, Some(1)), Some(&mut facts))
            .map(|_| ()),
        rule(BoardRuleError::RunOpen(1))
    );
    assert_eq!(
        (facts.plan_state_from, facts.plan_state_to),
        (Some(PlanState::Draft), None)
    );
}

/// A held task's plan is its holder's to decide; a set-aside run of it
/// holds the decision, since it may be the open one.
#[test]
fn a_held_tasks_plan_is_its_holders_and_a_set_aside_run_holds_it() {
    let held = |fields: TaskFields| TaskFields {
        status: TaskStatus::InProgress,
        claim: Some(Claim {
            holder: person("Ada"),
            since: at("09:00:00"),
            expires: at("11:00:00"),
        }),
        ..fields
    };
    let store = seeded(held, vec![plan_of("a", 1, PlanState::Draft)], vec![]);
    assert_eq!(
        accept_as(&store, "Bob").execute(&of(1, None)).map(|_| ()),
        rule(BoardRuleError::NotHolder)
    );
    let key = ArtifactKey::Run(slug("a"), 1);
    store.quarantine_key(key.clone(), "runs/a.1.md", "unreadable");
    assert_eq!(
        accept_as(&store, "Ada").execute(&of(1, None)).map(|_| ()),
        Err(ProjectBoardError::Quarantined {
            key,
            path: "runs/a.1.md".into(),
            object: crate::infrastructure::test_support::memory_board_store::memory_object(
                "runs/a.1.md"
            ),
        })
    );
}

#[test]
fn a_stale_expected_updated_is_refused_as_plan_changed() {
    let store = seeded(
        naming(None),
        vec![plan_of("a", 1, PlanState::Draft)],
        vec![],
    );
    let stale = PlanAcceptance {
        expected: ReadPin::Updated(at("08:00:00")),
        ..of(1, None)
    };
    assert_eq!(
        accept_as(&store, "Ada").execute(&stale).map(|_| ()),
        rule(BoardRuleError::PlanChanged {
            version: 1,
            updated: at("09:00:00"),
        })
    );
    assert_eq!(states(&store), [(1, PlanState::Draft)]);
}

/// An acceptance that loses the race to an edit of the same draft is
/// judged again on the fresh board: the edit landed, so it is refused,
/// never accepted unread.
#[test]
fn an_acceptance_that_loses_to_an_edit_of_its_draft_is_refused() {
    use crate::application::project_board::dto::PlanEdit;
    use crate::application::project_board::use_cases::EditPlan;
    let store = seeded(
        naming(None),
        vec![plan_of("a", 1, PlanState::Draft)],
        vec![],
    );
    let other = store.client();
    store.after_next_read(move |_| {
        let edit = PlanEdit {
            handoff: None,
            entry: PlanEntry::Legacy,
            task: slug("a"),
            version: 1,
            approach: Some("Edited.".into()),
            items: None,
            team: None,
            expected: None,
        };
        let editor = EditPlan::new(Arc::new(other), SetClock::at("09:30:00")).unwrap();
        editor.execute(&edit).unwrap();
    });
    let mut facts = OpFacts::default();
    assert_eq!(
        accept_as(&store, "Ada")
            .execute_noted(&of(1, None), Some(&mut facts))
            .map(|_| ()),
        rule(BoardRuleError::PlanChanged {
            version: 1,
            updated: at("09:30:00"),
        })
    );
    assert_eq!((facts.attempts, facts.lost), (Some(2), Some(1)));
    assert_eq!(states(&store), [(1, PlanState::Draft)]);
}

/// Plans that contradict each other are refused as such, never built on
/// and never a panic.
#[test]
fn plans_that_contradict_each_other_are_refused() {
    let unnamed = seeded(
        naming(None),
        vec![
            plan_of("a", 1, PlanState::Accepted),
            plan_of("a", 2, PlanState::Draft),
        ],
        vec![],
    );
    let two = seeded(
        naming(Some(1)),
        vec![
            plan_of("a", 1, PlanState::Accepted),
            plan_of("a", 2, PlanState::Accepted),
            plan_of("a", 3, PlanState::Draft),
        ],
        vec![],
    );
    for (store, version, seen, why) in [
        (
            unnamed,
            2,
            None,
            "version 1 is accepted; the task names none",
        ),
        (
            two,
            3,
            Some(1),
            "versions 1, 2 are accepted; the task names version 1",
        ),
    ] {
        let writes = store.writes();
        assert_eq!(
            accept_as(&store, "Ada")
                .execute(&of(version, seen))
                .map(|_| ()),
            rule(BoardRuleError::PlansInconsistent(why.into()))
        );
        assert_eq!(store.writes(), writes);
    }
}

/// Review round 1: Bob read draft 2 while the task named none; Ada then
/// accepted draft 3. Bob's acceptance of draft 2, its own draft unchanged,
/// is refused: it would supersede an acceptance he never saw. A rollback
/// that names what it supersedes is a knowing choice, and lands.
#[test]
fn a_stale_reader_never_supersedes_an_acceptance_it_did_not_see() {
    let store = seeded(
        naming(None),
        vec![
            plan_of("a", 2, PlanState::Draft),
            plan_of("a", 3, PlanState::Draft),
        ],
        vec![],
    );
    let bobs_read = of(2, None);
    accept_as(&store, "Ada").execute(&of(3, None)).unwrap();
    let writes = store.writes();
    assert_eq!(
        accept_as(&store, "Bob").execute(&bobs_read).map(|_| ()),
        rule(BoardRuleError::AcceptedPlanChanged { accepted: Some(3) })
    );
    assert_eq!(store.writes(), writes, "nothing written");
    assert_eq!(
        states(&store),
        [(2, PlanState::Draft), (3, PlanState::Accepted)]
    );
    let rollback = of(2, Some(3));
    let rolled_back = accept_as(&store, "Bob").execute(&rollback).unwrap();
    assert_eq!(
        rolled_back.superseded.map(|plan| plan.fields().version),
        Some(3)
    );
    assert_eq!(
        states(&store),
        [(2, PlanState::Accepted), (3, PlanState::Superseded)]
    );
}

/// Review round 1 (L1): a retry refused before its read notes no state
/// from the attempt that lost.
#[test]
fn a_retry_refused_before_its_read_notes_no_earlier_state() {
    let store = seeded(
        naming(None),
        vec![plan_of("a", 1, PlanState::Draft)],
        vec![],
    );
    store.after_next_read(|store| store.quarantine("a", "unreadable"));
    let mut facts = OpFacts::default();
    let refused = accept_as(&store, "Ada").execute_noted(&of(1, None), Some(&mut facts));
    assert!(
        matches!(refused, Err(ProjectBoardError::Quarantined { .. })),
        "{refused:?}"
    );
    assert_eq!((facts.attempts, facts.lost), (Some(2), Some(1)));
    assert_eq!((facts.plan_state_from, facts.plan_state_to), (None, None));
}

/// #2705 B9: a task stamped ahead of the outside time is built on, as
/// for every task write (a stamp is for display; the board's revision
/// orders changes), and its stamp never runs backwards.
#[test]
fn a_task_stamped_ahead_is_accepted_on_and_its_stamp_holds() {
    let store = seeded(
        naming(None),
        vec![plan_of("a", 1, PlanState::Draft)],
        vec![],
    );
    let client = store.client_as(person("Ada"));
    let behind = SetClock::with_reference("08:59:00", Some("08:59:00"));
    let accepting = AcceptPlan::new(Arc::new(client), behind).unwrap();
    let accepted = accepting.execute(&of(1, None)).unwrap();
    assert_eq!(accepted.task.fields().updated, at("09:00:00"));
}

/// Review round 2 (LOW-4): Ada accepts version 3 between Bob's read and
/// his write. Bob's acceptance of version 2, pinned to none accepted, is
/// judged again on the fresh board and refused: he never supersedes an
/// acceptance he did not see, and writes nothing.
#[test]
fn an_acceptance_racing_another_is_refused_on_its_retry() {
    let store = seeded(
        naming(None),
        vec![
            plan_of("a", 2, PlanState::Draft),
            plan_of("a", 3, PlanState::Draft),
        ],
        vec![],
    );
    let ada = store.client_as(person("Ada"));
    store.after_next_read(move |_| {
        let accepting = AcceptPlan::new(Arc::new(ada), SetClock::at("09:30:00")).unwrap();
        accepting.execute(&of(3, None)).unwrap();
    });
    let mut facts = OpFacts::default();
    assert_eq!(
        accept_as(&store, "Bob")
            .execute_noted(&of(2, None), Some(&mut facts))
            .map(|_| ()),
        rule(BoardRuleError::AcceptedPlanChanged { accepted: Some(3) })
    );
    assert_eq!((facts.attempts, facts.lost), (Some(2), Some(1)));
    let board = store.read().unwrap();
    assert_eq!(
        store.committed_by(&board.head).map(|writer| writer.name),
        Some("Ada".to_string()),
        "the last write is Ada's: Bob wrote nothing"
    );
    assert_eq!(
        states(&store),
        [(2, PlanState::Draft), (3, PlanState::Accepted)]
    );
}

/// Review round 2 (LOW-3): a stale reader of a done task hears it is
/// done, before its pin is judged.
#[test]
fn a_stale_reader_of_a_done_task_hears_it_is_done() {
    let done = |fields: TaskFields| TaskFields {
        status: TaskStatus::Done,
        resolution: Some(
            crate::domain::project_board::value_objects::vocabulary::Resolution::Completed,
        ),
        accepted_plan: Some(1),
        ..fields
    };
    let store = seeded(
        done,
        vec![
            plan_of("a", 1, PlanState::Accepted),
            plan_of("a", 2, PlanState::Draft),
        ],
        vec![],
    );
    assert_eq!(
        accept_as(&store, "Ada").execute(&of(2, None)).map(|_| ()),
        rule(BoardRuleError::TaskFinished(TaskStatus::Done))
    );
}
