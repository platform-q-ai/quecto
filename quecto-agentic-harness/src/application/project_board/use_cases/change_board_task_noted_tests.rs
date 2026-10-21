//! Epic #2481 T4: what a noted change saw, of the task as it last read
//! it: a write that lost once is two attempts, one lost, and the pause
//! between them on the clock's monotonic time.
use super::*;
use crate::application::project_board::telemetry::OpFacts;
use crate::domain::project_board::value_objects::telemetry::ClaimFacts;
use std::time::Duration;

/// A renew at 10:20 that loses once to a move to in progress.
fn renew_losing_once(store: &MemoryBoardStore) -> (ChangeBoardTask, Arc<SetClock>) {
    seed(store, held_by_ada(TaskStatus::Claimed));
    store.before_next_write(|board| {
        let other =
            ChangeBoardTask::new(Arc::new(board.client()), SetClock::at("10:10:00")).unwrap();
        other
            .execute(&slug("a"), &BoardOp::Move(TaskStatus::InProgress))
            .unwrap();
    });
    let clock = SetClock::at("10:20:00");
    *clock.tick.lock().unwrap() = Duration::from_millis(40);
    let renew = ChangeBoardTask::new(Arc::new(store.client()), clock.clone()).unwrap();
    (renew, clock)
}

#[test]
fn a_change_that_lost_once_tells_two_attempts_and_the_task_it_built_on() {
    let store = board();
    let (renew, _) = renew_losing_once(&store);
    let mut facts = OpFacts::default();
    renew
        .execute_noted(&slug("a"), &BoardOp::Renew, Some(&mut facts))
        .unwrap();
    let claim = ClaimFacts {
        held_s: 80 * 60,
        left_s: 40 * 60,
        holder_is_writer: true,
    };
    let want = OpFacts {
        status_from: Some(TaskStatus::InProgress),
        status_to: Some(TaskStatus::InProgress),
        claim: Some(claim),
        attempts: Some(2),
        lost: Some(1),
        pause_ms: Some(40),
        ..OpFacts::default()
    };
    assert_eq!(facts, want);
}

/// Another writer moves the board before each of the next `times`
/// writes is judged (it writes the task back as it is).
fn lose(store: &MemoryBoardStore, times: usize) {
    if times == 0 {
        return;
    }
    store.before_next_write(move |board| {
        let read = board.read().unwrap();
        board
            .write_tasks(&read.head, &read.tasks, "another writer")
            .unwrap();
        lose(board, times - 1);
    });
}

fn renew_noted(
    store: &MemoryBoardStore,
    clock: Arc<SetClock>,
) -> (Result<Task, ProjectBoardError>, OpFacts) {
    *clock.tick.lock().unwrap() = Duration::from_millis(40);
    let renew = ChangeBoardTask::new(Arc::new(store.client()), clock).unwrap();
    let mut facts = OpFacts::default();
    let renewed = renew.execute_noted(&slug("a"), &BoardOp::Renew, Some(&mut facts));
    (renewed, facts)
}

#[test]
fn each_pause_is_added_and_a_contended_change_tells_every_attempt_lost() {
    let store = board();
    seed(&store, held_by_ada(TaskStatus::Claimed));
    lose(&store, 2);
    let (renewed, facts) = renew_noted(&store, SetClock::at("10:20:00"));
    assert!(renewed.is_ok(), "{renewed:?}");
    assert_eq!(
        (facts.attempts, facts.lost, facts.pause_ms),
        (Some(3), Some(2), Some(80))
    );
    lose(&store, 3);
    // Past halfway again, so the renewal is due (#2656).
    let (contended, facts) = renew_noted(&store, SetClock::at("11:30:00"));
    assert_eq!(contended, Err(ProjectBoardError::Contended { attempts: 3 }));
    assert_eq!(
        (facts.attempts, facts.lost, facts.pause_ms),
        (Some(3), Some(3), Some(80))
    );
    assert_eq!(facts.status_from, Some(TaskStatus::Claimed), "as last read");
}

/// Owner condition: off means nothing is timed.
#[test]
fn an_unnoted_change_reads_no_monotonic_time() {
    let store = board();
    let (renew, clock) = renew_losing_once(&store);
    renew.execute(&slug("a"), &BoardOp::Renew).unwrap();
    assert_eq!(*clock.pauses.lock().unwrap(), [2], "it did lose once");
    assert_eq!(*clock.elapsed.lock().unwrap(), Duration::ZERO);
}

#[test]
fn a_refused_move_tells_its_target_and_the_claim_it_found() {
    let store = board();
    seed(&store, held_by_ada(TaskStatus::InProgress));
    let mut facts = OpFacts::default();
    let refused = change(&store, "Bob", "11:00:30").execute_noted(
        &slug("a"),
        &BoardOp::Move(TaskStatus::Review),
        Some(&mut facts),
    );
    assert!(
        matches!(refused, Err(ProjectBoardError::Rule(_))),
        "{refused:?}"
    );
    let claim = ClaimFacts {
        held_s: 2 * 60 * 60 + 30,
        left_s: -30,
        holder_is_writer: false,
    };
    let want = OpFacts {
        status_from: Some(TaskStatus::InProgress),
        status_to: Some(TaskStatus::Review),
        claim: Some(claim),
        attempts: Some(1),
        lost: Some(0),
        ..OpFacts::default()
    };
    assert_eq!(facts, want);
}

/// Review round 2: a later attempt that finds no task keeps nothing of an
/// earlier attempt's read (its first write loses to a writer that sets
/// the task's file aside).
mod a_task_gone_on_retry {
    use super::*;
    use crate::application::project_board::dto::DraftUpdate;
    use crate::application::project_board::use_cases::UpdateDraftTask;

    fn spoiled_before_the_first_write(store: &MemoryBoardStore) {
        store.before_next_write(|board| board.quarantine("a", "spoiled"));
    }

    fn assert_nothing_read(facts: &OpFacts) {
        assert_eq!((facts.status_from, facts.claim), (None, None), "{facts:?}");
        assert_eq!((facts.attempts, facts.lost), (Some(2), Some(1)));
    }

    #[test]
    fn a_change_tells_no_status_or_claim() {
        let store = board();
        seed(&store, held_by_ada(TaskStatus::Claimed));
        spoiled_before_the_first_write(&store);
        let mut facts = OpFacts::default();
        let renewed = change(&store, "Ada", "10:20:00").execute_noted(
            &slug("a"),
            &BoardOp::Renew,
            Some(&mut facts),
        );
        assert_eq!(renewed, Err(set_aside_task("a")));
        assert_nothing_read(&facts);
        assert_eq!(facts.status_to, None);
    }

    #[test]
    fn a_draft_edit_tells_no_status() {
        let store = board();
        seed(
            &store,
            TaskFields {
                status: TaskStatus::Draft,
                ..fields("a")
            },
        );
        spoiled_before_the_first_write(&store);
        let update = DraftUpdate {
            id: slug("a"),
            title: Some("Edited".into()),
            description: None,
            parent: None,
            depends_on: None,
            expected_updated: None,
        };
        let edit =
            UpdateDraftTask::new(Arc::new(store.client()), SetClock::at("10:20:00")).unwrap();
        let mut facts = OpFacts::default();
        assert_eq!(
            edit.execute_noted(&update, Some(&mut facts)),
            Err(set_aside_task("a"))
        );
        assert_nothing_read(&facts);
    }

    #[test]
    fn a_completion_tells_no_status_or_completion() {
        let store = board();
        seed(
            &store,
            TaskFields {
                prs: vec![7],
                ..held_by_ada(TaskStatus::InProgress)
            },
        );
        let host = MemoryPullRequests::default();
        host.set(7, PullRequestState::Merged);
        spoiled_before_the_first_write(&store);
        let complete = CompleteMergedTask::new(
            Arc::new(store.client()),
            SetClock::at("10:20:00"),
            Arc::new(host),
        )
        .unwrap();
        let mut facts = OpFacts::default();
        assert_eq!(
            complete.execute_noted(&slug("a"), Some(&mut facts)),
            Err(set_aside_task("a"))
        );
        assert_nothing_read(&facts);
        assert_eq!(facts.completion, None);
    }
}
