use super::CompleteMergedTask;
use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::{
    ProjectBoardStore, PullRequestState, PullRequestStatesError,
};
use crate::application::project_board::use_cases::fixtures_tests::{
    SetClock, at, board, fields, person, slug,
};
use crate::domain::project_board::entities::claim::Claim;
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::entities::task_parts::Run;
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::vocabulary::{RunOutcome, TaskStatus};
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

/// Whoever holds the task, its merge closes it: the claim is cleared and
/// a run still open is ended as lost when the task is done.
#[test]
fn a_merge_closes_a_held_task_and_ends_its_open_run_as_lost() {
    let store = board();
    let held = Task::new(TaskFields {
        status: TaskStatus::InProgress,
        claim: Some(Claim {
            holder: person("Carol"),
            since: at("09:00:00"),
            expires: at("11:00:00"),
        }),
        runs: vec![Run {
            id: slug("r1"),
            started: at("09:05:00"),
            ended: None,
            outcome: None,
        }],
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
    let run = &done.fields().runs[0];
    assert_eq!(
        (
            done.fields().status,
            done.fields().claim.clone(),
            run.ended.clone(),
            run.outcome
        ),
        (
            TaskStatus::Done,
            None,
            Some(at("10:00:00")),
            Some(RunOutcome::Lost)
        )
    );
}
