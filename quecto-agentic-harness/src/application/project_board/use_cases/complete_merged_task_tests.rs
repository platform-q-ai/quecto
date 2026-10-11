use super::CompleteMergedTask;
use crate::application::project_board::dto::{BoardOp, ProjectBoardError};
use crate::application::project_board::ports::{
    ProjectBoardStore, PullRequestState, PullRequestStatesError,
};
use crate::application::project_board::use_cases::ChangeBoardTask;
use crate::application::project_board::use_cases::fixtures_tests::{
    SetClock, at, board, fields, person, slug,
};
use crate::domain::project_board::entities::claim::Claim;
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use crate::infrastructure::test_support::memory_pull_requests::MemoryPullRequests;
use std::sync::Arc;

/// Task `a`, in review, held by Ada, linking `prs`.
fn seeded(prs: &[u64]) -> MemoryBoardStore {
    let store = board();
    let task = Task::new(TaskFields {
        status: TaskStatus::Review,
        claim: Some(Claim {
            holder: person("Ada"),
            since: at("09:00:00"),
            expires: at("11:00:00"),
        }),
        prs: prs.to_vec(),
        created: at("08:00:00"),
        updated: at("09:00:00"),
        ..fields("a")
    })
    .unwrap();
    let head = store.read().unwrap().head;
    store.write(&head, &[task], "seed").unwrap();
    store
}

/// The system's completion, run by Bob's machine, reading `host`.
fn complete(store: &MemoryBoardStore, host: &MemoryPullRequests) -> CompleteMergedTask {
    CompleteMergedTask::new(
        Arc::new(store.client_as(person("Bob"))),
        SetClock::at("10:00:00"),
        Arc::new(host.clone()),
    )
    .unwrap()
}

fn status(store: &MemoryBoardStore) -> TaskStatus {
    store.read().unwrap().tasks[0].fields().status
}

/// Each merge event asks again: the first merge of two leaves the task
/// in review, the second closes it.
#[test]
fn a_task_linking_two_pull_requests_is_done_when_the_second_merges() {
    let store = seeded(&[7, 8]);
    let host = MemoryPullRequests::default();
    host.set(7, PullRequestState::Merged);
    host.set(8, PullRequestState::Open);
    let writes = store.writes();
    let first = complete(&store, &host).execute(&slug("a"));
    assert_eq!(
        first,
        Err(ProjectBoardError::Rule(BoardRuleError::PrsUnmerged(vec![
            8
        ])))
    );
    assert_eq!(
        (status(&store), store.writes()),
        (TaskStatus::Review, writes)
    );
    host.set(8, PullRequestState::Merged);
    let done = complete(&store, &host).execute(&slug("a")).unwrap();
    assert_eq!(
        (done.fields().status, done.fields().claim.clone()),
        (TaskStatus::Done, None)
    );
}

/// An open, a closed-unmerged and an unknown pull request each keep the
/// task as it is, with nothing written; so does a host that cannot be
/// read.
#[test]
fn a_pull_request_not_read_as_merged_completes_nothing() {
    for (state, refused) in [
        (
            PullRequestState::Open,
            ProjectBoardError::Rule(BoardRuleError::PrsUnmerged(vec![9])),
        ),
        (
            PullRequestState::Closed,
            ProjectBoardError::Rule(BoardRuleError::PrsUnmerged(vec![9])),
        ),
        (
            PullRequestState::Unknown,
            ProjectBoardError::PrStateUnknown(9),
        ),
    ] {
        let store = seeded(&[7, 9]);
        let host = MemoryPullRequests::default();
        host.set(7, PullRequestState::Merged);
        host.set(9, state);
        let writes = store.writes();
        assert_eq!(
            complete(&store, &host).execute(&slug("a")),
            Err(refused),
            "{state:?}"
        );
        assert_eq!(store.writes(), writes, "{state:?}");
    }
    let store = seeded(&[7]);
    let host = MemoryPullRequests::default();
    host.set(7, PullRequestState::Merged);
    host.fail("the host hung up");
    let writes = store.writes();
    assert_eq!(
        complete(&store, &host).execute(&slug("a")),
        Err(ProjectBoardError::PullRequests(
            PullRequestStatesError::Unavailable("the host hung up".into())
        ))
    );
    assert_eq!(store.writes(), writes);
}

/// A holder released the task with its pull request linked: it is ready,
/// held by no one, and its merge still closes it.
#[test]
fn a_released_task_is_done_when_its_pull_request_merges() {
    let store = board();
    let ready = Task::new(TaskFields {
        prs: vec![7],
        created: at("08:00:00"),
        updated: at("09:00:00"),
        ..fields("a")
    })
    .unwrap();
    let head = store.read().unwrap().head;
    store.write(&head, &[ready], "seed").unwrap();
    let host = MemoryPullRequests::default();
    host.set(7, PullRequestState::Merged);
    let done = complete(&store, &host).execute(&slug("a")).unwrap();
    assert_eq!(done.fields().status, TaskStatus::Done);
}

/// Whoever holds the task, its merge closes it: the claim is cleared.
#[test]
fn a_merge_closes_a_held_task_whoever_holds_it() {
    let store = board();
    let held = Task::new(TaskFields {
        status: TaskStatus::InProgress,
        claim: Some(Claim {
            holder: person("Carol"),
            since: at("09:00:00"),
            expires: at("11:00:00"),
        }),
        prs: vec![7],
        created: at("08:00:00"),
        updated: at("09:05:00"),
        ..fields("a")
    })
    .unwrap();
    let head = store.read().unwrap().head;
    store.write(&head, &[held], "seed").unwrap();
    let host = MemoryPullRequests::default();
    host.set(7, PullRequestState::Merged);
    let done = complete(&store, &host).execute(&slug("a")).unwrap();
    assert_eq!(
        (done.fields().status, done.fields().claim.clone()),
        (TaskStatus::Done, None)
    );
}

/// A task the rule would not complete is refused before the host is
/// asked: a done task linking a pull request the host does not know is
/// still `NotAllowed`, not unknown.
#[test]
fn a_task_that_cannot_complete_is_refused_before_the_host_is_read() {
    let store = board();
    let done = Task::new(TaskFields {
        status: TaskStatus::Done,
        prs: vec![7],
        created: at("08:00:00"),
        updated: at("09:00:00"),
        ..fields("a")
    })
    .unwrap();
    let head = store.read().unwrap().head;
    store.write(&head, &[done], "seed").unwrap();
    let host = MemoryPullRequests::default();
    host.fail("must not be asked");
    assert_eq!(
        complete(&store, &host).execute(&slug("a")),
        Err(ProjectBoardError::Rule(BoardRuleError::NotAllowed {
            from: TaskStatus::Done,
            to: TaskStatus::Done
        }))
    );
}

/// The pull request had already merged when Ada linked it, so no merge
/// event follows; running the completion after the link closes the task,
/// whoever runs it (S2 runs it after every link, on a move to review, on
/// merge events, and as anyone's "complete").
#[test]
fn a_pull_request_merged_before_it_was_linked_completes_the_task() {
    let store = seeded(&[]);
    let host = MemoryPullRequests::default();
    host.set(7, PullRequestState::Merged);
    ChangeBoardTask::new(
        Arc::new(store.client_as(person("Ada"))),
        SetClock::at("09:30:00"),
    )
    .unwrap()
    .execute(&slug("a"), &BoardOp::AddPr(7))
    .unwrap();
    let done = complete(&store, &host).execute(&slug("a")).unwrap();
    assert_eq!(done.fields().status, TaskStatus::Done);
}

/// Ada links pull request 8 between the completion's read and its write:
/// the write is lost, the completion is built again on the fresh task, and
/// the host is asked again, now about 8 too.
#[test]
fn a_pull_request_linked_during_the_completion_is_asked_about() {
    let store = seeded(&[7]);
    let host = MemoryPullRequests::default();
    host.set(7, PullRequestState::Merged);
    host.set(8, PullRequestState::Open);
    store.before_next_write(|board| {
        ChangeBoardTask::new(
            Arc::new(board.client_as(person("Ada"))),
            SetClock::at("09:30:00"),
        )
        .unwrap()
        .execute(&slug("a"), &BoardOp::AddPr(8))
        .unwrap();
    });
    let writes = store.writes();
    assert_eq!(
        complete(&store, &host).execute(&slug("a")),
        Err(ProjectBoardError::Rule(BoardRuleError::PrsUnmerged(vec![
            8
        ])))
    );
    assert_eq!(host.asked(), [7, 7, 8], "asked again on the fresh task");
    assert_eq!(
        (status(&store), store.writes()),
        (TaskStatus::Review, writes + 1),
        "only Ada's link landed"
    );
}

/// Once a merge closed the task and ended its run as lost, its former
/// holder has no claim to end a run under.
/// The reads share one budget: once it is spent no further pull request
/// is asked about, and the completion is refused as the host's failure.
#[test]
fn the_reads_stop_once_their_budget_is_spent() {
    let store = seeded(&[7, 8]);
    let host = MemoryPullRequests::default();
    host.set(7, PullRequestState::Merged);
    host.set(8, PullRequestState::Merged);
    let spent = complete(&store, &host)
        .with_read_budget(std::time::Duration::ZERO)
        .execute(&slug("a"));
    let Err(ProjectBoardError::PullRequests(PullRequestStatesError::Unavailable(why))) = spent
    else {
        panic!("{spent:?}")
    };
    assert!(why.contains("longer than 0 s"), "{why}");
    assert_eq!(host.asked(), Vec::<u64>::new());
    assert_eq!(status(&store), TaskStatus::Review);
    let done = complete(&store, &host).execute(&slug("a"));
    assert!(done.is_ok(), "{done:?}");
    assert_eq!(host.asked(), vec![7, 8]);
}

/// The budget spans every attempt: a write lost to another writer is
/// tried again, but its reads count against the same deadline, measured on
/// the board clock's monotonic time.
#[test]
fn a_lost_write_does_not_restart_the_budget() {
    let store = seeded(&[7]);
    let host = MemoryPullRequests::default();
    host.set(7, PullRequestState::Merged);
    let clock = SetClock::at("10:00:00");
    *clock.tick.lock().unwrap() = std::time::Duration::from_secs(20);
    store.before_next_write(|store| {
        // Another writer moves the board first.
        let other = Task::new(TaskFields {
            created: at("08:00:00"),
            updated: at("08:00:00"),
            ..fields("b")
        })
        .unwrap();
        let head = store.read().unwrap().head;
        store.write(&head, &[other], "another writer").unwrap();
    });
    let completion = CompleteMergedTask::new(
        Arc::new(store.client_as(person("Bob"))),
        clock.clone(),
        Arc::new(host.clone()),
    )
    .unwrap()
    .with_read_budget(std::time::Duration::from_secs(30));
    let spent = completion.execute(&slug("a"));
    assert!(
        matches!(
            &spent,
            Err(ProjectBoardError::PullRequests(PullRequestStatesError::Unavailable(why)))
                if why.contains("longer than 30 s")
        ),
        "{spent:?}"
    );
    assert_eq!(host.asked(), vec![7], "read once, on the first attempt");
    assert_eq!(
        clock.pauses.lock().unwrap().len(),
        1,
        "the write was tried again"
    );
}
