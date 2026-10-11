use super::*;
use crate::application::project_board::dto::NewBoardTaskStatus;
use crate::application::project_board::operations::query_listings::QueryListings;
use crate::application::project_board::ports::PullRequestState;
use crate::application::project_board::use_cases::QueryBoard;
use crate::application::project_board::use_cases::fixtures_tests::{SetClock, new_task, slug};
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use crate::infrastructure::test_support::memory_pull_requests::MemoryPullRequests;

fn operations(
    store: &MemoryBoardStore,
    clock: Arc<SetClock>,
    prs: Result<Arc<dyn PullRequestStates>, String>,
) -> BoardOperations {
    let listings = Arc::new(QueryListings(QueryBoard::new(Arc::new(store.client()))));
    BoardOperations::new(Arc::new(store.client()), clock, listings, prs).unwrap()
}

fn ready_task(id: &str) -> BoardRequest {
    BoardRequest::Create(NewBoardTask {
        status: NewBoardTaskStatus::Ready,
        ..new_task(id)
    })
}

#[test]
fn linking_a_merged_pull_request_or_moving_to_review_completes_the_task() {
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
    let BoardOutcome::Changed {
        completion: Some(completion),
        ..
    } = open
    else {
        panic!("{open:?}")
    };
    let Err(refusal) = *completion else {
        panic!("not completed")
    };
    assert!(matches!(refusal, ProjectBoardError::Rule(_)), "{refusal:?}");
    host.set(8, PullRequestState::Merged);
    let review = ops
        .execute(&BoardRequest::Change(
            a.clone(),
            BoardOp::Move(TaskStatus::Review),
        ))
        .unwrap();
    let BoardOutcome::Changed {
        completion: Some(completion),
        ..
    } = review
    else {
        panic!("{review:?}")
    };
    let Ok(done) = *completion else {
        panic!("completed")
    };
    assert_eq!(done.fields().status, TaskStatus::Done);
    let renew = ops.execute(&BoardRequest::Show(a)).unwrap();
    assert!(matches!(renew, BoardOutcome::Shown { .. }));
}

#[test]
fn a_clock_note_only_for_changes_that_stamp_tasks_with_no_outside_time() {
    let store = MemoryBoardStore::default();
    let ops = operations(&store, SetClock::at("10:00:00"), Err("no host".into()));
    let create = ready_task("a");
    assert_eq!(ops.clock_note(&create), Some("set without one".into()));
    for request in [
        BoardRequest::Bootstrap,
        BoardRequest::ListReady,
        BoardRequest::Clear {
            path: "tasks/x.json".into(),
            keep: None,
        },
    ] {
        assert_eq!(ops.clock_note(&request), None, "{request:?}");
    }
    let referenced = operations(
        &store,
        SetClock::with_reference("10:00:00", Some("10:00:00")),
        Err("no host".into()),
    );
    assert_eq!(referenced.clock_note(&create), None);
}

#[test]
fn show_tells_a_task_set_aside_from_one_not_on_the_board() {
    let store = MemoryBoardStore::default();
    let ops = operations(&store, SetClock::at("10:00:00"), Err("no host".into()));
    ops.execute(&BoardRequest::Bootstrap).unwrap();
    ops.execute(&ready_task("b")).unwrap();
    store.quarantine("b", "malformed");
    assert_eq!(
        ops.execute(&BoardRequest::Show(slug("b"))),
        Err(ProjectBoardError::Quarantined(slug("b")))
    );
    assert_eq!(
        ops.execute(&BoardRequest::Show(slug("c"))),
        Err(ProjectBoardError::NotFound(slug("c")))
    );
    let complete = ops.execute(&BoardRequest::Complete(slug("b")));
    assert!(
        matches!(complete, Err(ProjectBoardError::PullRequests(_))),
        "{complete:?}"
    );
}
