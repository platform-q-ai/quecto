use crate::domain::project_board::services::transitions::MoveReason;
use std::sync::Arc;

use super::*;
use crate::application::project_board::dto::{NewBoardTask, NewBoardTaskStatus};
use crate::application::project_board::operations::query_listings::{QueryListings, assembled};
use crate::application::project_board::operations::{NoPullRequestHost, PlanView};
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
        BoardRequest::Change(
            a.clone(),
            BoardOp::Move(TaskStatus::Claimed, MoveReason::Absent),
        ),
        BoardRequest::Change(
            a.clone(),
            BoardOp::Move(TaskStatus::InProgress, MoveReason::Absent),
        ),
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
            BoardOp::Move(TaskStatus::Review, MoveReason::Absent),
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
    let shown = ops
        .execute(&BoardRequest::Show(a, PlanView::default()))
        .unwrap();
    let BoardOutcome::Shown(shown, _) = shown else {
        panic!("{shown:?}")
    };
    assert_eq!(
        shown.task.fields().status,
        TaskStatus::Done,
        "the board holds it"
    );
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
        BoardRequest::Change(
            a.clone(),
            BoardOp::Move(TaskStatus::Claimed, MoveReason::Absent),
        ),
        BoardRequest::Change(
            a.clone(),
            BoardOp::Move(TaskStatus::InProgress, MoveReason::Absent),
        ),
    ] {
        ops.execute(&request).unwrap();
    }
    let review = ops
        .execute(&BoardRequest::Change(
            a,
            BoardOp::Move(TaskStatus::Review, MoveReason::Absent),
        ))
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
        ops.execute(&BoardRequest::Show(slug("b"), PlanView::default())),
        Err(set_aside_task("b"))
    );
    assert_eq!(
        ops.execute(&BoardRequest::Show(slug("c"), PlanView::default())),
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
    // #2658: a task that links no pull request has nothing to ask a host
    // about: it is refused by the completion rule itself, as with a host.
    ops.execute(&ready_task("d")).unwrap();
    assert_eq!(
        ops.execute(&BoardRequest::Complete(slug("d"))),
        Err(ProjectBoardError::Rule(BoardRuleError::NoPrs))
    );
}

/// #2658: with no pull request host, only a task that links a pull
/// request needs one to complete.
#[test]
fn a_complete_with_no_host_needs_one_only_for_pull_requests() {
    let store = MemoryBoardStore::default();
    let ops = operations(&store, SetClock::at("10:00:00"), no_host());
    ops.execute(&BoardRequest::Bootstrap).unwrap();
    let e = slug("e");
    for request in [
        ready_task("e"),
        BoardRequest::Change(
            e.clone(),
            BoardOp::Move(TaskStatus::Claimed, MoveReason::Absent),
        ),
        BoardRequest::Change(
            e.clone(),
            BoardOp::Move(TaskStatus::InProgress, MoveReason::Absent),
        ),
        BoardRequest::Change(e.clone(), BoardOp::AddPr(8)),
    ] {
        ops.execute(&request).unwrap();
    }
    assert_eq!(
        ops.execute(&BoardRequest::Complete(e)),
        Err(ProjectBoardError::NoPullRequestHost(
            NoPullRequestHost::NotAPullRequestHost
        ))
    );
}

/// A store that hands over every read with one plan of task `a` twice, as
/// a store that broke its contract would: whatever reads the plans
/// refuses it, and whatever reads the task alone does not.
struct PlanTwice(MemoryBoardStore);

impl crate::application::project_board::ports::ProjectBoardStore for PlanTwice {
    fn read(
        &self,
    ) -> Result<
        crate::application::project_board::ports::BoardSnapshot,
        crate::application::project_board::ports::BoardStoreError,
    > {
        let mut board = self.0.read()?;
        let plan = crate::infrastructure::test_support::board_artifacts::plan("a", 1);
        board.plans.extend([plan.clone(), plan]);
        Ok(board)
    }

    fn write(
        &self,
        expected: &crate::application::project_board::ports::BoardHead,
        artifacts: &[crate::application::project_board::ports::BoardArtifact],
        message: &str,
    ) -> Result<
        crate::application::project_board::ports::BoardWrite,
        crate::application::project_board::ports::BoardStoreError,
    > {
        self.0.write(expected, artifacts, message)
    }

    fn clear_set_aside(
        &self,
        expected: &crate::application::project_board::ports::BoardHead,
        clear: &crate::application::project_board::ports::SetAsideClear,
        message: &str,
    ) -> Result<
        crate::application::project_board::ports::BoardWrite,
        crate::application::project_board::ports::BoardStoreError,
    > {
        self.0.clear_set_aside(expected, clear, message)
    }

    fn bootstrap(
        &self,
    ) -> Result<
        crate::application::project_board::ports::BoardBootstrap,
        crate::application::project_board::ports::BoardStoreError,
    > {
        self.0.bootstrap()
    }

    fn writer(&self) -> crate::domain::project_board::entities::claim::Identity {
        self.0.writer()
    }
}

/// Review round 2: with no pull request host, `complete` never reads a
/// task's plans, so a plan held twice does not stand in for its answer:
/// a task linking no pull request is the completion rule's (#2658).
#[test]
fn a_complete_with_no_host_looks_the_task_up_alone() {
    let memory = MemoryBoardStore::default();
    let ops = operations(&memory, SetClock::at("10:00:00"), no_host());
    ops.execute(&BoardRequest::Bootstrap).unwrap();
    ops.execute(&ready_task("a")).unwrap();
    let store: Arc<dyn crate::application::project_board::ports::ProjectBoardStore> =
        Arc::new(PlanTwice(memory.client()));
    let listings = Arc::new(QueryListings(QueryBoard::new(Arc::clone(&store))));
    let ops = assembled(store, SetClock::at("10:00:00"), listings, no_host()).unwrap();
    assert!(matches!(
        ops.execute(&BoardRequest::Show(slug("a"), PlanView::default())),
        Err(ProjectBoardError::Duplicate { .. })
    ));
    assert_eq!(
        ops.execute(&BoardRequest::Complete(slug("a"))),
        Err(ProjectBoardError::Rule(BoardRuleError::NoPrs))
    );
}

/// Epic #2481 T4: what the use cases saw of a noted request.
mod noted {
    use super::*;
    use crate::application::project_board::dto::DraftUpdate;
    use crate::application::project_board::ports::ProjectBoardStore;
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
        // A write's commit is the board's (#2663), told apart in its own
        // test (`a_write_notes_its_commit_and_no_lock_race`).
        facts.head = None;
        (result, facts)
    }

    fn written_once(facts: OpFacts) -> OpFacts {
        OpFacts {
            attempts: Some(1),
            lost: Some(0),
            ..facts
        }
    }

    /// #2663: a write notes the commit it made, and how many lock races
    /// its fetches met (none, in memory); a refused one notes no commit.
    #[test]
    fn a_write_notes_its_commit_and_no_lock_race() {
        let store = MemoryBoardStore::default();
        let ops = operations(&store, SetClock::at("10:00:00"), no_host());
        ops.execute(&BoardRequest::Bootstrap).unwrap();
        let mut facts = OpFacts::default();
        ops.execute_noted(&ready_task("a"), Some(&mut facts))
            .unwrap();
        let head = store.read().unwrap().head;
        assert_eq!(
            (facts.head, facts.lock_races, facts.attempts),
            (Some(head), None, Some(1))
        );
        let mut refused = OpFacts::default();
        assert!(
            ops.execute_noted(&ready_task("a"), Some(&mut refused))
                .is_err()
        );
        assert_eq!((refused.head, refused.lock_races), (None, None));
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
            description: "## Goal\r\nWhy.\r\n\r\n## Done when\r\n- [ ] How. ".into(),
            status: NewBoardTaskStatus::Draft,
            ..new_task("a")
        });
        let (created, facts) = noted(&ops, &create);
        assert!(created.is_ok(), "{created:?}");
        let normalised = Normalised {
            title: Some(vec![TextChange::EdgeSpace]),
            description: Some(vec![TextChange::Crlf, TextChange::TrailingSpace]),
            approach: None,
            items: None,
        };
        let want = OpFacts {
            status_to: Some(TaskStatus::Draft),
            normalised: Some(normalised),
            ..OpFacts::default()
        };
        assert_eq!(facts, written_once(want));
        // A refused create wrote no task, so it tells no status.
        let (taken, facts) = noted(&ops, &create);
        assert_eq!(taken, Err(ProjectBoardError::Exists(slug("a"))));
        assert_eq!((facts.status_to, facts.attempts), (None, Some(1)));
        let edit = BoardRequest::UpdateDraft(DraftUpdate {
            id: slug("a"),
            title: None,
            description: Some("## Goal\nPlain.\n\n## Done when\n- [ ] Done".into()),
            parent: None,
            depends_on: None,
            expected_updated: None,
            entry: crate::domain::project_board::entities::task_parts::Entry::Sectioned,
        });
        let (edited, facts) = noted(&ops, &edit);
        assert!(edited.is_ok(), "{edited:?}");
        let want = OpFacts {
            status_from: Some(TaskStatus::Draft),
            status_to: Some(TaskStatus::Draft),
            normalised: Some(Normalised {
                title: None,
                description: Some(vec![]),
                approach: None,
                items: None,
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
            entry: crate::domain::project_board::entities::task_parts::Entry::Sectioned,
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
            BoardRequest::Change(
                a.clone(),
                BoardOp::Move(TaskStatus::Claimed, MoveReason::Absent),
            ),
            BoardRequest::Change(
                a.clone(),
                BoardOp::Move(TaskStatus::InProgress, MoveReason::Absent),
            ),
        ] {
            ops.execute(&request).unwrap();
        }
        let completion = |kind, prs_linked, prs_pending| CompletionFacts {
            completion: kind,
            completion_code: None,
            completion_rule: None,
            prs_linked,
            prs_pending,
            completion_attempts: None,
            completion_races_lost: None,
            completion_pause_ms: None,
        };
        // After a link, the completion's own write: one attempt, none lost.
        let written_once_more = |facts: CompletionFacts| CompletionFacts {
            completion_attempts: Some(1),
            completion_races_lost: Some(0),
            ..facts
        };
        let (_, linked) = noted(&ops, &BoardRequest::Change(a.clone(), BoardOp::AddPr(8)));
        // Claimed at 10:00:01 (a stamp a second past the create), read at
        // 10:00:00 by this set clock: held for no time, never less (#2662).
        let claim = ClaimFacts {
            held_s: 0,
            left_s: 7201,
            holder_is_writer: true,
        };
        let want = OpFacts {
            status_from: Some(TaskStatus::InProgress),
            status_to: Some(TaskStatus::InProgress),
            claim: Some(claim),
            completion: Some(written_once_more(completion(
                CompletionKind::Pending,
                1,
                Some(1),
            ))),
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
        let failed_completion = written_once_more(completion(CompletionKind::Failed, 2, None));
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

    /// The completion after a link counts its own write apart: here it
    /// loses once to another writer, while the link's own write wins first
    /// time.
    #[test]
    fn a_completion_counts_its_own_write_apart_from_the_change() {
        let store = MemoryBoardStore::default();
        let host = MemoryPullRequests::default();
        host.set(8, PullRequestState::Merged);
        let clock = SetClock::at("10:00:00");
        let ops = operations(&store, clock.clone(), Ok(Arc::new(host)));
        let a = slug("a");
        for request in [
            BoardRequest::Bootstrap,
            ready_task("a"),
            BoardRequest::Change(
                a.clone(),
                BoardOp::Move(TaskStatus::Claimed, MoveReason::Absent),
            ),
            BoardRequest::Change(
                a.clone(),
                BoardOp::Move(TaskStatus::InProgress, MoveReason::Absent),
            ),
        ] {
            ops.execute(&request).unwrap();
        }
        *clock.tick.lock().unwrap() = std::time::Duration::from_millis(25);
        // Armed at the link's write, so it moves the board before the
        // completion's write is judged.
        store.before_next_write(|board| {
            board.before_next_write(|board| {
                let read = board.read().unwrap();
                board
                    .write_tasks(&read.head, &read.tasks, "another writer")
                    .unwrap();
            });
        });
        let (linked, facts) = noted(&ops, &BoardRequest::Change(a, BoardOp::AddPr(8)));
        let Ok(BoardOutcome::Changed {
            completion: Some(Completion::Done(_)),
            ..
        }) = linked
        else {
            panic!("{linked:?}")
        };
        assert_eq!(
            (facts.attempts, facts.lost, facts.pause_ms),
            (Some(1), Some(0), None),
            "the link's own write"
        );
        let completion = facts.completion.unwrap();
        assert_eq!(
            (
                completion.completion,
                completion.completion_attempts,
                completion.completion_races_lost,
                completion.completion_pause_ms
            ),
            (CompletionKind::Done, Some(2), Some(1), Some(25))
        );
    }

    /// `complete` records only done or pending: a task the rule will not
    /// complete records no completion.
    #[test]
    fn a_complete_refused_otherwise_tells_no_completion() {
        let store = MemoryBoardStore::default();
        let host = MemoryPullRequests::default();
        let ops = operations(&store, SetClock::at("10:00:00"), Ok(Arc::new(host)));
        ops.execute(&BoardRequest::Bootstrap).unwrap();
        let draft = BoardRequest::Create(NewBoardTask {
            status: NewBoardTaskStatus::Draft,
            ..new_task("a")
        });
        ops.execute(&draft).unwrap();
        let (refused, facts) = noted(&ops, &BoardRequest::Complete(slug("a")));
        assert!(
            matches!(
                refused,
                Err(ProjectBoardError::Rule(BoardRuleError::NotAllowed { .. }))
            ),
            "{refused:?}"
        );
        assert_eq!(facts.completion, None);
        assert_eq!(facts.status_from, Some(TaskStatus::Draft));
    }

    /// A clear tells its attempts: here one lost to another writer.
    #[test]
    fn a_clear_tells_its_attempts() {
        let store = MemoryBoardStore::default();
        let clock = SetClock::at("10:00:00");
        *clock.tick.lock().unwrap() = std::time::Duration::from_millis(30);
        let ops = operations(&store, clock, no_host());
        ops.execute(&BoardRequest::Bootstrap).unwrap();
        ops.execute(&ready_task("z")).unwrap();
        store.set_aside_path("notes.txt", "not a task file");
        let object = store.read().unwrap().quarantined[0].object.clone();
        store.before_next_write(|board| {
            let read = board.read().unwrap();
            board
                .write_tasks(&read.head, &read.tasks, "another writer")
                .unwrap();
        });
        let clear = BoardRequest::Clear {
            path: "notes.txt".into(),
            object,
            repeated: Default::default(),
        };
        let (cleared, facts) = noted(&ops, &clear);
        assert!(cleared.is_ok(), "{cleared:?}");
        let want = OpFacts {
            attempts: Some(2),
            lost: Some(1),
            pause_ms: Some(30),
            ..OpFacts::default()
        };
        assert_eq!(facts, want);
    }

    /// A show tells how many plan versions and runs of its task it gave
    /// (#2481 M4a), counts only; a refused show tells none.
    #[test]
    fn a_show_tells_how_many_plans_and_runs_it_gave() {
        use crate::application::project_board::ports::BoardArtifact;
        use crate::infrastructure::test_support::board_artifacts::{plan, run};
        let store = MemoryBoardStore::default();
        let ops = operations(&store, SetClock::at("10:00:00"), no_host());
        ops.execute(&BoardRequest::Bootstrap).unwrap();
        ops.execute(&ready_task("a")).unwrap();
        let (_, facts) = noted(&ops, &BoardRequest::Show(slug("a"), PlanView::default()));
        let none = OpFacts {
            plans: Some(0),
            runs: Some(0),
            ..OpFacts::default()
        };
        assert_eq!(facts, none);
        let head = store.read().unwrap().head;
        let seeded = [
            BoardArtifact::Plan(plan("a", 2)),
            BoardArtifact::Plan(plan("a", 1)),
            BoardArtifact::Run(run("a", 1)),
            BoardArtifact::Plan(plan("b", 1)),
        ];
        store.write(&head, &seeded, "board: seed").unwrap();
        let (shown, facts) = noted(&ops, &BoardRequest::Show(slug("a"), PlanView::default()));
        assert!(shown.is_ok(), "{shown:?}");
        let want = OpFacts {
            plans: Some(2),
            runs: Some(1),
            ..OpFacts::default()
        };
        assert_eq!(facts, want);
        let (refused, facts) = noted(&ops, &BoardRequest::Show(slug("b"), PlanView::default()));
        assert_eq!(refused, Err(ProjectBoardError::NotFound(slug("b"))));
        assert_eq!(facts, OpFacts::default());
        // #2481 M4b: a show of a named version records it, found or not.
        let named = |version| PlanView {
            version: Some(version),
            page: None,
        };
        let (shown, facts) = noted(&ops, &BoardRequest::Show(slug("a"), named(2)));
        assert!(shown.is_ok(), "{shown:?}");
        assert_eq!(facts.plan_version, Some(2));
        let (unknown, facts) = noted(&ops, &BoardRequest::Show(slug("a"), named(7)));
        assert!(unknown.is_err(), "{unknown:?}");
        assert_eq!((facts.plan_version, facts.plans), (Some(7), Some(2)));
    }
}
