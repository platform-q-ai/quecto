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
            path: "tasks/x.md".into(),
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

/// Epic #2481 T4: what the use cases saw of a noted request.
mod noted {
    use super::*;
    use crate::application::project_board::dto::DraftUpdate;
    use crate::application::project_board::telemetry::OpFacts;
    use crate::domain::project_board::value_objects::telemetry::{
        ClaimFacts, CompletionFacts, CompletionKind, Normalised, TextChange,
    };

    fn noted(
        ops: &RunBoardOperation,
        request: &BoardRequest,
    ) -> (Result<BoardOutcome, ProjectBoardError>, OpFacts) {
        let mut facts = OpFacts::default();
        let result = ops.execute_noted(request, Some(&mut facts));
        (result, facts)
    }

    fn written_once(facts: OpFacts) -> OpFacts {
        OpFacts {
            attempts: Some(1),
            lost: Some(0),
            ..facts
        }
    }

    #[test]
    fn a_create_and_a_draft_edit_tell_what_the_normaliser_changed() {
        let store = MemoryBoardStore::default();
        let ops = operations(&store, SetClock::at("10:00:00"), no_host());
        let (bootstrapped, facts) = noted(&ops, &BoardRequest::Bootstrap);
        assert!(bootstrapped.is_ok());
        assert_eq!(facts, OpFacts::default(), "a bootstrap writes no task");
        let create = BoardRequest::Create(NewBoardTask {
            title: "  A title".into(),
            description: "Why.\r\nHow. ".into(),
            status: NewBoardTaskStatus::Draft,
            ..new_task("a")
        });
        let (created, facts) = noted(&ops, &create);
        assert!(created.is_ok(), "{created:?}");
        let normalised = Normalised {
            title: Some(vec![TextChange::EdgeSpace]),
            description: Some(vec![TextChange::Crlf, TextChange::TrailingSpace]),
        };
        let want = OpFacts {
            status_to: Some(TaskStatus::Draft),
            normalised: Some(normalised),
            ..OpFacts::default()
        };
        assert_eq!(facts, written_once(want));
        let edit = BoardRequest::UpdateDraft(DraftUpdate {
            id: slug("a"),
            title: None,
            description: Some("Plain.".into()),
            parent: None,
            depends_on: None,
            expected_updated: None,
        });
        let (edited, facts) = noted(&ops, &edit);
        assert!(edited.is_ok(), "{edited:?}");
        let want = OpFacts {
            status_from: Some(TaskStatus::Draft),
            status_to: Some(TaskStatus::Draft),
            normalised: Some(Normalised {
                title: None,
                description: Some(vec![]),
            }),
            ..OpFacts::default()
        };
        assert_eq!(facts, written_once(want));
        let untitled = BoardRequest::UpdateDraft(DraftUpdate {
            id: slug("a"),
            title: None,
            description: None,
            parent: Some(None),
            depends_on: None,
            expected_updated: None,
        });
        let (_, facts) = noted(&ops, &untitled);
        assert_eq!(facts.normalised, None, "no text given, none told");
    }

    /// A link tells what came of completing its task, and `complete`
    /// tells done or pending; neither names a failed one's code.
    #[test]
    fn a_link_and_a_complete_tell_what_came_of_completing_the_task() {
        let store = MemoryBoardStore::default();
        let host = MemoryPullRequests::default();
        host.set(8, PullRequestState::Open);
        let ops = operations(&store, SetClock::at("10:00:00"), Ok(Arc::new(host.clone())));
        let a = slug("a");
        for request in [
            BoardRequest::Bootstrap,
            ready_task("a"),
            BoardRequest::Change(a.clone(), BoardOp::Move(TaskStatus::Claimed)),
            BoardRequest::Change(a.clone(), BoardOp::Move(TaskStatus::InProgress)),
        ] {
            ops.execute(&request).unwrap();
        }
        let completion = |kind, prs_linked, prs_pending| CompletionFacts {
            completion: kind,
            completion_code: None,
            prs_linked,
            prs_pending,
        };
        let (_, linked) = noted(&ops, &BoardRequest::Change(a.clone(), BoardOp::AddPr(8)));
        // Claimed at 10:00:01 (a stamp a second past the create), read at
        // 10:00:00 by this set clock.
        let claim = ClaimFacts {
            held_s: -1,
            left_s: 7201,
            holder_is_writer: true,
        };
        let want = OpFacts {
            status_from: Some(TaskStatus::InProgress),
            status_to: Some(TaskStatus::InProgress),
            claim: Some(claim),
            completion: Some(completion(CompletionKind::Pending, 1, Some(1))),
            ..OpFacts::default()
        };
        assert_eq!(linked, written_once(want));
        let (refused, pending) = noted(&ops, &BoardRequest::Complete(a.clone()));
        assert!(refused.is_err());
        let want = OpFacts {
            status_from: Some(TaskStatus::InProgress),
            completion: Some(completion(CompletionKind::Pending, 1, Some(1))),
            ..OpFacts::default()
        };
        assert_eq!(pending, written_once(want));
        host.set(9, PullRequestState::Unknown);
        let (_, failed) = noted(&ops, &BoardRequest::Change(a.clone(), BoardOp::AddPr(9)));
        let failed_completion = completion(CompletionKind::Failed, 2, None);
        assert_eq!(failed.completion, Some(failed_completion));
        host.set(8, PullRequestState::Merged);
        host.set(9, PullRequestState::Merged);
        let (done, facts) = noted(&ops, &BoardRequest::Complete(a));
        assert!(done.is_ok(), "{done:?}");
        let want = OpFacts {
            status_from: Some(TaskStatus::InProgress),
            status_to: Some(TaskStatus::Done),
            completion: Some(completion(CompletionKind::Done, 2, None)),
            ..OpFacts::default()
        };
        assert_eq!(facts.completion, want.completion);
        assert_eq!(
            (facts.status_from, facts.status_to, facts.attempts),
            (want.status_from, want.status_to, Some(1))
        );
    }
}
