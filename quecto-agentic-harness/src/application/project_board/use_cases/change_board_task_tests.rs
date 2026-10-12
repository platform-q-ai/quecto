use super::ChangeBoardTask;
use crate::application::project_board::dto::{BoardOp, ProjectBoardError};
use crate::application::project_board::ports::{ProjectBoardStore, PullRequestState};
use crate::application::project_board::use_cases::CompleteMergedTask;
use crate::application::project_board::use_cases::fixtures_tests::{
    SetClock, at, board, fields, person, set_aside_task, slug,
};
use crate::domain::project_board::entities::claim::{Claim, Identity};
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use crate::infrastructure::test_support::memory_pull_requests::MemoryPullRequests;
use std::sync::Arc;

fn seed(store: &MemoryBoardStore, fields: TaskFields) {
    let task = Task::new(TaskFields {
        created: at("08:00:00"),
        updated: at("09:10:00"),
        ..fields
    })
    .unwrap();
    let head = store.read().unwrap().head;
    store.write_tasks(&head, &[task], "seed").unwrap();
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
    ChangeBoardTask::new(Arc::new(store.client_as(person(who))), SetClock::at(time)).unwrap()
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
        Err(ProjectBoardError::Rule(BoardRuleError::PrsLinked(vec![7]))),
        "a linked PR closes it, not its holder"
    );
    let host = MemoryPullRequests::default();
    host.set(7, PullRequestState::Merged);
    let system = CompleteMergedTask::new(
        Arc::new(store.client_as(person("Bob"))),
        SetClock::at("10:20:00"),
        Arc::new(host),
    )
    .unwrap();
    let done = system.execute(&a).unwrap();
    assert_eq!(
        (done.fields().status, done.fields().claim.clone()),
        (TaskStatus::Done, None),
        "the system's move, whoever runs it"
    );
}

#[test]
fn a_claim_is_renewed_taken_over_and_a_pull_request_unlinked() {
    let store = board();
    seed(
        &store,
        TaskFields {
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
    assert_eq!(set_aside, Err(set_aside_task("q")));
    assert_eq!(store.writes(), writes);
}

/// Ada's other machine starts the work while this one renews: the renewal
/// is made again on the started task and keeps its progress.
#[test]
fn a_lost_change_is_made_again_on_the_fresh_task() {
    let store = board();
    seed(&store, held_by_ada(TaskStatus::Claimed));
    store.before_next_write(|board| {
        let other =
            ChangeBoardTask::new(Arc::new(board.client()), SetClock::at("10:10:00")).unwrap();
        other
            .execute(&slug("a"), &BoardOp::Move(TaskStatus::InProgress))
            .unwrap();
    });
    let clock = SetClock::at("10:20:00");
    let renew = ChangeBoardTask::new(Arc::new(store.client()), clock.clone()).unwrap();
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
        )
        .unwrap();
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
    )
    .unwrap();
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

/// The holder unlinks a pull request deliberately (a wrong or abandoned
/// one); with none linked, Review→Done is theirs.
#[test]
fn the_holder_unlinks_a_pull_request_and_then_finishes_the_task() {
    let store = board();
    seed(
        &store,
        TaskFields {
            prs: vec![4],
            ..held_by_ada(TaskStatus::Review)
        },
    );
    let a = slug("a");
    let early = change(&store, "Ada", "10:00:00").execute(&a, &BoardOp::Move(TaskStatus::Done));
    assert_eq!(
        early,
        Err(ProjectBoardError::Rule(BoardRuleError::PrsLinked(vec![4])))
    );
    change(&store, "Ada", "10:01:00")
        .execute(&a, &BoardOp::RemovePr(4))
        .unwrap();
    let done = change(&store, "Ada", "10:02:00")
        .execute(&a, &BoardOp::Move(TaskStatus::Done))
        .unwrap();
    assert_eq!(done.fields().status, TaskStatus::Done);
}

/// The bound on a task stamped ahead is inclusive, as the clock's own is:
/// a task stamped exactly the reference plus 60 s plus the stamp's slack
/// (30 s) is built on; a second later it is not.
#[test]
fn a_task_stamped_at_the_bound_is_built_on_and_a_second_past_it_is_not() {
    let built_on = |updated: &str| {
        let store = board();
        let task = Task::new(TaskFields {
            created: at("08:00:00"),
            updated: at(updated),
            ..held_by_ada(TaskStatus::Claimed)
        })
        .unwrap();
        let head = store.read().unwrap().head;
        store.write_tasks(&head, &[task], "seed").unwrap();
        ChangeBoardTask::new(
            Arc::new(store.client()),
            SetClock::with_reference("09:00:30", Some("09:00:00")),
        )
        .unwrap()
        .execute(&slug("a"), &BoardOp::Renew)
    };
    let at_bound = built_on("09:01:30").unwrap();
    assert_eq!(
        at_bound.fields().updated,
        at("09:01:31"),
        "a second past the task it built on"
    );
    assert_eq!(
        built_on("09:01:31"),
        Err(ProjectBoardError::TaskAhead {
            id: slug("a"),
            updated: at("09:01:31")
        })
    );
}

/// A reference the board cannot add 60 s to is refused before anything
/// is built.
#[test]
fn a_reference_at_the_end_of_time_is_out_of_range() {
    let store = board();
    seed(&store, held_by_ada(TaskStatus::Claimed));
    let end = Timestamp::parse("9999-12-31T23:59:30Z").unwrap();
    let refused = ChangeBoardTask::new(
        Arc::new(store.client()),
        SetClock::reading(end.clone(), Some(end.clone())),
    )
    .unwrap()
    .execute(&slug("a"), &BoardOp::Renew);
    assert_eq!(refused, Err(ProjectBoardError::ClockOutOfRange(end)));
}

/// A store writing as an identity no task can name is refused when the
/// use cases are built, before any change.
#[test]
fn a_store_with_an_invalid_writer_is_refused_when_built() {
    let store = board();
    let nobody = Identity {
        name: " ".into(),
        email: "not an email".into(),
    };
    let invalid = || Arc::new(store.client_as(nobody.clone()));
    assert!(matches!(
        ChangeBoardTask::new(invalid(), SetClock::at("10:00:00")),
        Err(ProjectBoardError::InvalidWriter(_))
    ));
    assert!(matches!(
        CompleteMergedTask::new(
            invalid(),
            SetClock::at("10:00:00"),
            Arc::new(MemoryPullRequests::default())
        ),
        Err(ProjectBoardError::InvalidWriter(_))
    ));
    assert!(matches!(
        super::super::CreateBoardTask::new(invalid(), SetClock::at("10:00:00")),
        Err(ProjectBoardError::InvalidWriter(_))
    ));
}

/// A client 55 s fast (inside the 60 s bound) renews a dozen times in a
/// row; each stamp is a second past the last, edging past the bound, yet
/// a writer whose clock is right still builds on the task.
#[test]
fn quick_writes_from_a_fast_clock_never_lock_out_a_right_one() {
    let store = board();
    let task = Task::new(TaskFields {
        created: at("08:00:00"),
        updated: at("09:00:00"),
        ..held_by_ada(TaskStatus::InProgress)
    })
    .unwrap();
    let head = store.read().unwrap().head;
    store.write_tasks(&head, &[task], "seed").unwrap();
    let fast = ChangeBoardTask::new(
        Arc::new(store.client_as(person("Ada"))),
        SetClock::with_reference("09:00:55", Some("09:00:00")),
    )
    .unwrap();
    for _ in 0..12 {
        fast.execute(&slug("a"), &BoardOp::Renew).unwrap();
    }
    assert!(
        current(&store).updated > at("09:01:00"),
        "past the bound itself"
    );
    let right = ChangeBoardTask::new(
        Arc::new(store.client_as(person("Ada"))),
        SetClock::with_reference("09:00:00", Some("09:00:00")),
    )
    .unwrap();
    let renewed = right.execute(&slug("a"), &BoardOp::Renew);
    assert!(renewed.is_ok(), "{renewed:?}");
}

/// A pull request another task on the board links is not linked again.
#[test]
fn a_pull_request_another_task_links_is_not_linked_again() {
    let store = board();
    seed(&store, held_by_ada(TaskStatus::Claimed));
    seed(
        &store,
        TaskFields {
            prs: vec![7],
            ..fields("b")
        },
    );
    let writes = store.writes();
    let refused = change(&store, "Ada", "10:00:00").execute(&slug("a"), &BoardOp::AddPr(7));
    assert_eq!(
        refused,
        Err(ProjectBoardError::Rule(
            BoardRuleError::ChangeAlreadyLinked {
                task: slug("b"),
                change: None
            }
        ))
    );
    assert_eq!(store.writes(), writes);
    let linked = change(&store, "Ada", "10:00:00").execute(&slug("a"), &BoardOp::AddPr(8));
    assert_eq!(linked.map(|task| task.fields().prs.clone()), Ok(vec![8]));
}

/// While another task's file is set aside its pull requests are unknown,
/// so a link is refused rather than risk linking one twice; the task's
/// other ops go on.
#[test]
fn a_pull_request_is_not_linked_while_a_tasks_file_is_set_aside() {
    let store = board();
    seed(&store, held_by_ada(TaskStatus::Claimed));
    seed(
        &store,
        TaskFields {
            prs: vec![7],
            ..fields("b")
        },
    );
    store.quarantine("b", "malformed");
    let writes = store.writes();
    let refused = change(&store, "Ada", "10:00:00").execute(&slug("a"), &BoardOp::AddPr(7));
    assert_eq!(refused, Err(set_aside_task("b")));
    assert_eq!(store.writes(), writes);
    let renewed = change(&store, "Ada", "10:00:00").execute(&slug("a"), &BoardOp::Renew);
    assert!(renewed.is_ok(), "{renewed:?}");
}

/// Of several carried files set aside, a link names the least, whatever
/// the store's order: one whose task cannot be told first, then by key.
#[test]
fn a_refused_link_names_the_least_carried_file_set_aside() {
    let store = board();
    seed(&store, held_by_ada(TaskStatus::Claimed));
    seed(&store, fields("c"));
    seed(&store, fields("b"));
    store.quarantine("c", "malformed");
    store.quarantine("b", "malformed");
    let link = || change(&store, "Ada", "10:00:00").execute(&slug("a"), &BoardOp::AddPr(7));
    assert_eq!(link(), Err(set_aside_task("b")));
    store.set_aside_path("tasks/Z Not A Slug.json", "malformed");
    assert!(
        matches!(link(), Err(ProjectBoardError::SetAsideFile { path, .. }) if path == "tasks/Z Not A Slug.json"),
        "a file of no known task is named first"
    );
}

/// A set-aside file every write carries whose task cannot be told may
/// hold any pull request, so a link waits for it too; a stray the next
/// write drops blocks nothing.
#[test]
fn a_pull_request_is_not_linked_while_a_carried_file_of_no_known_task_is_set_aside() {
    let store = board();
    seed(&store, held_by_ada(TaskStatus::Claimed));
    store.set_aside_stray("tasks/notes.txt", "not a task file");
    let linked = change(&store, "Ada", "10:00:00").execute(&slug("a"), &BoardOp::AddPr(6));
    assert!(linked.is_ok(), "a stray blocks nothing: {linked:?}");
    store.set_aside_path("tasks/Not A Slug.json", "malformed");
    let object = store
        .read()
        .unwrap()
        .quarantined
        .last()
        .unwrap()
        .object
        .clone();
    let writes = store.writes();
    let refused = change(&store, "Ada", "10:00:00").execute(&slug("a"), &BoardOp::AddPr(7));
    assert_eq!(
        refused,
        Err(ProjectBoardError::SetAsideFile {
            path: "tasks/Not A Slug.json".into(),
            object
        })
    );
    assert_eq!(store.writes(), writes);
}
