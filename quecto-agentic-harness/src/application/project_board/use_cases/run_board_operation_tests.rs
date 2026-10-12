use std::sync::Arc;

use super::*;
use crate::application::project_board::dto::{NewBoardTask, NewBoardTaskStatus};
use crate::application::project_board::operations::NoPullRequestHost;
use crate::application::project_board::operations::query_listings::{QueryListings, assembled};
use crate::application::project_board::ports::{PullRequestState, PullRequestStates};
use crate::application::project_board::use_cases::QueryBoard;
use crate::application::project_board::use_cases::fixtures_tests::{
    SetClock, new_task, set_aside_task, slug,
};
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use crate::infrastructure::test_support::memory_pull_requests::MemoryPullRequests;

fn operations(
    store: &MemoryBoardStore,
    clock: Arc<SetClock>,
    prs: Result<Arc<dyn PullRequestStates>, NoPullRequestHost>,
) -> RunBoardOperation {
    let listings = Arc::new(QueryListings(QueryBoard::new(Arc::new(store.client()))));
    assembled(Arc::new(store.client()), clock, listings, prs).unwrap()
}

fn no_host() -> Result<Arc<dyn PullRequestStates>, NoPullRequestHost> {
    Err(NoPullRequestHost::NotAPullRequestHost)
}

fn ready_task(id: &str) -> BoardRequest {
    BoardRequest::Create(NewBoardTask {
        status: NewBoardTaskStatus::Ready,
        ..new_task(id)
    })
}

/// A completion waiting on pull requests not merged yet is pending, not an
/// error; once they merge, the next move to review completes the task.
#[test]
fn a_completion_is_pending_until_the_pull_requests_merge() {
    let store = MemoryBoardStore::default();
    let host = MemoryPullRequests::default();
    host.set(8, PullRequestState::Open);
    let ops = operations(
        &store,
        SetClock::with_reference("10:00:00", Some("10:00:00")),
        Ok(Arc::new(host.clone())),
    );
    let a = slug("a");
    for request in [
        BoardRequest::Bootstrap,
        ready_task("a"),
        BoardRequest::Change(a.clone(), BoardOp::Move(TaskStatus::Claimed)),
        BoardRequest::Change(a.clone(), BoardOp::Move(TaskStatus::InProgress)),
    ] {
        ops.execute(&request).unwrap();
    }
    let open = ops
        .execute(&BoardRequest::Change(a.clone(), BoardOp::AddPr(8)))
        .unwrap();
    let BoardOutcome::Changed { completion, .. } = open else {
        panic!("{open:?}")
    };
    assert_eq!(completion, Some(Completion::Pending(vec![8])));
    host.set(8, PullRequestState::Merged);
    let review = ops
        .execute(&BoardRequest::Change(
            a.clone(),
            BoardOp::Move(TaskStatus::Review),
        ))
        .unwrap();
    let BoardOutcome::Changed {
        completion: Some(Completion::Done(done)),
        ..
    } = review
    else {
        panic!("{review:?}")
    };
    assert_eq!(done.fields().status, TaskStatus::Done);
    let shown = ops.execute(&BoardRequest::Show(a)).unwrap();
    let BoardOutcome::Shown { task, .. } = shown else {
        panic!("{shown:?}")
    };
    assert_eq!(task.fields().status, TaskStatus::Done, "the board holds it");
}

/// A move to review of a task linking no pull request has nothing to
/// complete: no completion is reported.
#[test]
fn a_task_linking_no_pull_request_reports_no_completion() {
    let store = MemoryBoardStore::default();
    let ops = operations(&store, SetClock::at("10:00:00"), no_host());
    let a = slug("a");
    for request in [
        BoardRequest::Bootstrap,
        ready_task("a"),
        BoardRequest::Change(a.clone(), BoardOp::Move(TaskStatus::Claimed)),
        BoardRequest::Change(a.clone(), BoardOp::Move(TaskStatus::InProgress)),
    ] {
        ops.execute(&request).unwrap();
    }
    let review = ops
        .execute(&BoardRequest::Change(a, BoardOp::Move(TaskStatus::Review)))
        .unwrap();
    assert!(
        matches!(
            review,
            BoardOutcome::Changed {
                completion: None,
                ..
            }
        ),
        "{review:?}"
    );
}

#[test]
fn a_clock_note_only_for_requests_that_read_the_clock_with_no_outside_time() {
    let store = MemoryBoardStore::default();
    let ops = operations(&store, SetClock::at("10:00:00"), no_host());
    let create = ready_task("a");
    assert_eq!(ops.clock_note(&create), Some("set without one".into()));
    assert_eq!(
        ops.clock_note(&BoardRequest::ListTakeable),
        Some("set without one".into()),
        "claims are judged by the clock"
    );
    for request in [
        BoardRequest::Bootstrap,
        BoardRequest::ListReady,
        BoardRequest::Clear {
            path: "tasks/x.json".into(),
            object: "a".repeat(40),
            repeated: Default::default(),
        },
    ] {
        assert_eq!(ops.clock_note(&request), None, "{request:?}");
    }
    let referenced = operations(
        &store,
        SetClock::with_reference("10:00:00", Some("10:00:00")),
        no_host(),
    );
    assert_eq!(referenced.clock_note(&create), None);
}

#[test]
fn show_tells_a_task_set_aside_from_one_not_on_the_board() {
    let store = MemoryBoardStore::default();
    let ops = operations(&store, SetClock::at("10:00:00"), no_host());
    ops.execute(&BoardRequest::Bootstrap).unwrap();
    ops.execute(&ready_task("b")).unwrap();
    store.quarantine("b", "malformed");
    assert_eq!(
        ops.execute(&BoardRequest::Show(slug("b"))),
        Err(set_aside_task("b"))
    );
    assert_eq!(
        ops.execute(&BoardRequest::Show(slug("c"))),
        Err(ProjectBoardError::NotFound(slug("c")))
    );
    // With no pull request host, a task not on the board is answered as
    // such before the host is.
    assert_eq!(
        ops.execute(&BoardRequest::Complete(slug("c"))),
        Err(ProjectBoardError::NotFound(slug("c")))
    );
    assert_eq!(
        ops.execute(&BoardRequest::Complete(slug("b"))),
        Err(set_aside_task("b"))
    );
    ops.execute(&ready_task("d")).unwrap();
    assert_eq!(
        ops.execute(&BoardRequest::Complete(slug("d"))),
        Err(ProjectBoardError::NoPullRequestHost(
            NoPullRequestHost::NotAPullRequestHost
        ))
    );
}
