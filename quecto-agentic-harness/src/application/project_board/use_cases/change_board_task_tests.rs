use super::ChangeBoardTask;
use crate::application::project_board::dto::{BoardOp, ProjectBoardError};
use crate::application::project_board::ports::ProjectBoardStore;
use crate::application::project_board::use_cases::CompleteMergedTask;
use crate::application::project_board::use_cases::fixtures_tests::{
    SetClock, at, board, fields, person, slug,
};
use crate::domain::project_board::entities::claim::Claim;
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::entities::task_parts::Run;
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::vocabulary::{RunOutcome, TaskStatus};
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use std::collections::BTreeSet;
use std::sync::Arc;

fn seed(store: &MemoryBoardStore, fields: TaskFields) {
    let task = Task::new(TaskFields {
        created: at("08:00:00"),
        updated: at("09:10:00"),
        ..fields
    })
    .unwrap();
    let head = store.read().unwrap().head;
    store.write(&head, &[task], "seed").unwrap();
}

fn held_by_ada(status: TaskStatus) -> TaskFields {
    let claim = Claim {
        holder: person("Ada"),
        since: at("09:00:00"),
        expires: at("11:00:00"),
    };
    TaskFields {
        status,
        claim: Some(claim),
        ..fields("a")
    }
}

/// The change use case over `store`, writing as `who`.
fn change(store: &MemoryBoardStore, who: &str, time: &str) -> ChangeBoardTask {
    ChangeBoardTask::new(Arc::new(store.client_as(person(who))), SetClock::at(time))
}

fn current(store: &MemoryBoardStore) -> TaskFields {
    store.read().unwrap().tasks[0].fields().clone()
}

#[test]
fn a_task_is_claimed_worked_linked_and_completed_when_its_pr_merges() {
    let store = board();
    seed(&store, fields("a"));
    let a = slug("a");
    let claimed = change(&store, "Ada", "10:00:00")
        .execute(&a, &BoardOp::Move(TaskStatus::Claimed))
        .unwrap();
    let claim = claimed.fields().claim.clone().unwrap();
    assert_eq!(
        (claimed.fields().status, claim.holder, claim.expires),
        (TaskStatus::Claimed, person("Ada"), at("12:00:00"))
    );
    for (time, op) in [
        ("10:05:00", BoardOp::Move(TaskStatus::InProgress)),
        ("10:10:00", BoardOp::AddPr(7)),
        ("10:15:00", BoardOp::Move(TaskStatus::Review)),
    ] {
        change(&store, "Ada", time).execute(&a, &op).unwrap();
    }
    let early = change(&store, "Ada", "10:16:00").execute(&a, &BoardOp::Move(TaskStatus::Done));
    assert_eq!(
        early,
        Err(ProjectBoardError::Rule(BoardRuleError::PrsOpen(vec![7]))),
        "a linked PR closes it, not its holder"
    );
    let system = CompleteMergedTask::new(
        Arc::new(store.client_as(person("Bob"))),
        SetClock::at("10:20:00"),
    );
    let done = system.execute(&a, &BTreeSet::from([7])).unwrap();
    assert_eq!(
        (done.fields().status, done.fields().claim.clone()),
        (TaskStatus::Done, None),
        "the system's move, whoever runs it"
    );
}

#[test]
fn a_claim_is_renewed_taken_over_and_a_lost_run_ended() {
    let store = board();
    let run = Run {
        id: slug("r1"),
        started: at("09:05:00"),
        ended: None,
        outcome: None,
    };
    seed(
        &store,
        TaskFields {
            runs: vec![run],
            prs: vec![3],
            ..held_by_ada(TaskStatus::InProgress)
        },
    );
    let a = slug("a");
    let renewed = change(&store, "Ada", "10:30:00")
        .execute(&a, &BoardOp::Renew)
        .unwrap();
    assert_eq!(
        renewed.fields().claim.as_ref().unwrap().expires,
        at("12:30:00")
    );
    let taken = change(&store, "Bob", "13:00:00")
        .execute(&a, &BoardOp::TakeOver)
        .unwrap();
    assert_eq!(
        taken.fields().claim.as_ref().unwrap().holder,
        person("Bob"),
        "the store's committer acts"
    );
    let ended = change(&store, "Bob", "13:05:00")
        .execute(&a, &BoardOp::EndRunLost)
        .unwrap();
    assert_eq!(ended.fields().runs[0].outcome, Some(RunOutcome::Lost));
    change(&store, "Bob", "13:10:00")
        .execute(&a, &BoardOp::RemovePr(3))
        .unwrap();
    assert_eq!(current(&store).prs, Vec::<u64>::new());
}

#[test]
fn a_change_the_rules_refuse_writes_nothing() {
    let store = board();
    seed(&store, held_by_ada(TaskStatus::Claimed));
    store.quarantine("q", "malformed");
    let writes = store.writes();
    let refused = change(&store, "Bob", "10:00:00")
        .execute(&slug("a"), &BoardOp::Move(TaskStatus::InProgress));
    assert_eq!(
        refused,
        Err(ProjectBoardError::Rule(BoardRuleError::NotHolder))
    );
    let missing = change(&store, "Ada", "10:00:00").execute(&slug("zz"), &BoardOp::Renew);
    assert_eq!(missing, Err(ProjectBoardError::NotFound(slug("zz"))));
    let set_aside = change(&store, "Ada", "10:00:00").execute(&slug("q"), &BoardOp::Renew);
    assert_eq!(set_aside, Err(ProjectBoardError::Quarantined(slug("q"))));
    assert_eq!(store.writes(), writes);
}

/// Ada's other machine starts the work while this one renews: the renewal
/// is made again on the started task and keeps its progress.
#[test]
fn a_lost_change_is_made_again_on_the_fresh_task() {
    let store = board();
    seed(&store, held_by_ada(TaskStatus::Claimed));
    store.before_next_write(|board| {
        let other = ChangeBoardTask::new(Arc::new(board.client()), SetClock::at("10:10:00"));
        other
            .execute(&slug("a"), &BoardOp::Move(TaskStatus::InProgress))
            .unwrap();
    });
    let clock = SetClock::at("10:20:00");
    let renew = ChangeBoardTask::new(Arc::new(store.client()), clock.clone());
    let renewed = renew.execute(&slug("a"), &BoardOp::Renew).unwrap();
    assert_eq!(
        renewed.fields().status,
        TaskStatus::InProgress,
        "the other machine's move is kept"
    );
    assert_eq!(
        renewed.fields().claim.as_ref().unwrap().expires,
        at("12:20:00")
    );
    assert_eq!(
        *clock.pauses.lock().unwrap(),
        [2],
        "one pause before the second attempt"
    );
}

/// Ada claims the ready task first: Bob's claim, made again on the claimed
/// task, is refused, and Ada keeps it.
#[test]
fn a_change_no_longer_allowed_on_the_fresh_task_is_refused() {
    let store = board();
    seed(&store, fields("a"));
    store.before_next_write(|board| {
        let ada = ChangeBoardTask::new(
            Arc::new(board.client_as(person("Ada"))),
            SetClock::at("10:00:00"),
        );
        ada.execute(&slug("a"), &BoardOp::Move(TaskStatus::Claimed))
            .unwrap();
    });
    let refused =
        change(&store, "Bob", "10:00:01").execute(&slug("a"), &BoardOp::Move(TaskStatus::Claimed));
    let not_allowed = BoardRuleError::NotAllowed {
        from: TaskStatus::Claimed,
        to: TaskStatus::Claimed,
    };
    assert_eq!(refused, Err(ProjectBoardError::Rule(not_allowed)));
    assert_eq!(current(&store).claim.unwrap().holder, person("Ada"));
}

/// A task stamped past the reference by a clock that ran ahead is not
/// built on; the store's refusal is reported as it is.
#[test]
fn a_task_from_the_future_and_a_refusal_are_reported() {
    let store = board();
    seed(&store, held_by_ada(TaskStatus::Claimed));
    let skewed = ChangeBoardTask::new(
        Arc::new(store.client()),
        SetClock::with_reference("09:00:00", Some("09:00:00")),
    );
    let ahead = skewed.execute(&slug("a"), &BoardOp::Renew);
    assert_eq!(
        ahead,
        Err(ProjectBoardError::TaskAhead {
            id: slug("a"),
            updated: at("09:10:00")
        })
    );
    store.refuse_next_write("a hook said no");
    let refused = change(&store, "Ada", "10:00:00").execute(&slug("a"), &BoardOp::Renew);
    assert_eq!(
        refused,
        Err(ProjectBoardError::Refused("a hook said no".into()))
    );
}
