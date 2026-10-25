//! #2705 T5: an end_run writes its task's status in the same change as
//! the run: a success to review, a blocked stop to blocked, any other end
//! back to ready with the claim released.
use super::*;

fn task_of(store: &MemoryBoardStore) -> Task {
    store.read().unwrap().tasks[0].clone()
}

#[test]
fn a_succeeded_end_run_leaves_the_task_in_review_in_one_revision() {
    let (store, held) = stranded();
    let writes = store.writes();
    let done = [SwarmItemState::Completed; 3];
    let ended = ender_keeping(&store, Some(swarm(SwarmEnd::Succeeded, done)))
        .execute(&ending(RunEndOutcome::Succeeded))
        .unwrap();
    assert_eq!(store.writes(), writes + 1, "one revision");
    let task = task_of(&store);
    let fields = task.fields();
    assert_eq!(fields.status, TaskStatus::Review);
    assert_eq!(fields.claim, held.fields().claim, "its holder keeps it");
    assert_eq!(Some(&fields.updated), ended.run.fields().ended.as_ref());
    assert_eq!(ended.status, TaskStatus::Review, "the answer says where");
    assert_eq!(ended.run, run_1(&store));
}

#[test]
fn a_failed_end_run_leaves_the_task_ready_with_its_claim_released() {
    let (store, _) = stranded();
    let writes = store.writes();
    let ended = ender(&store, "Ada", "10:30:00")
        .execute(&ending(RunEndOutcome::Failed))
        .unwrap();
    assert_eq!(store.writes(), writes + 1, "one revision");
    let task = task_of(&store);
    assert_eq!(
        (task.fields().status, task.fields().claim.clone()),
        (TaskStatus::Ready, None)
    );
    assert_eq!(ended.status, TaskStatus::Ready);
    assert_eq!(ended.run.fields().outcome, Some(RunOutcome::Failed));
}

/// A stop goes back to ready, but one whose swarm ended blocked waits for
/// the user in blocked, its holder keeping the claim.
#[test]
fn a_stop_leaves_it_blocked_only_when_its_swarm_ended_blocked() {
    let todo = [SwarmItemState::Ready; 3];
    for (kept, want) in [
        (Some(swarm(SwarmEnd::Blocked, todo)), TaskStatus::Blocked),
        (Some(swarm(SwarmEnd::Cancelled, todo)), TaskStatus::Ready),
        (None, TaskStatus::Ready),
    ] {
        let (store, held) = stranded();
        ender_keeping(&store, kept)
            .execute(&ending(RunEndOutcome::Stopped))
            .unwrap();
        let task = task_of(&store);
        assert_eq!(task.fields().status, want);
        let claim = match want {
            TaskStatus::Blocked => held.fields().claim.clone(),
            _ => None,
        };
        assert_eq!(task.fields().claim, claim, "{want:?}");
    }
}

/// A run its holder cannot account for, lost mid-claim: back to ready.
#[test]
fn a_lost_end_run_leaves_the_task_ready_with_its_claim_released() {
    let (store, _) = stranded();
    ender(&store, "Ada", "10:30:00")
        .execute(&ending(RunEndOutcome::Lost))
        .unwrap();
    let task = task_of(&store);
    assert_eq!(
        (task.fields().status, task.fields().claim.clone()),
        (TaskStatus::Ready, None)
    );
}

/// The Pocket relay's Stop (quecto-pocket `stop_board_run`): its bot ends
/// the run as stopped through the CLI (no board kept), then reads the task
/// again and moves it back to ready only while it is still in progress or
/// claimed. The end leaves it ready, released, so the Stop moves nothing,
/// and the run's end is the one write.
#[test]
fn the_pocket_relays_stop_finds_the_task_ready_and_moves_nothing() {
    let (store, _) = stranded();
    let writes = store.writes();
    ender(&store, "Ada", "10:30:00")
        .execute(&ending(RunEndOutcome::Stopped))
        .unwrap();
    let task = task_of(&store);
    let fields = task.fields();
    let relay_moves = matches!(fields.status, TaskStatus::InProgress | TaskStatus::Claimed);
    assert!(
        !relay_moves,
        "the relay's allowlist moves nothing: {fields:?}"
    );
    assert_eq!(
        (fields.status, fields.claim.clone()),
        (TaskStatus::Ready, None)
    );
    assert!(store.read().unwrap().runs.iter().all(|run| !run.is_open()));
    assert_eq!(store.writes(), writes + 1);
}

/// #2705 T5 review round 1: with no board of its swarm kept (the CLI), a
/// stop goes where its caller's `stop` says: blocked keeps the task with
/// its holder, cancelled (and none given) releases it.
#[test]
fn a_stop_with_no_board_kept_goes_where_its_caller_says() {
    use crate::domain::project_board::services::run_outcomes::StopKind;
    for (stop, want) in [
        (Some(StopKind::Blocked), TaskStatus::Blocked),
        (Some(StopKind::Cancelled), TaskStatus::Ready),
        (None, TaskStatus::Ready),
    ] {
        let (store, held) = stranded();
        let request = RunEnd {
            stop,
            ..ending(RunEndOutcome::Stopped)
        };
        let ended = ender(&store, "Ada", "10:30:00").execute(&request).unwrap();
        assert_eq!(ended.status, want, "{stop:?}");
        let claim = match want {
            TaskStatus::Blocked => held.fields().claim.clone(),
            _ => None,
        };
        assert_eq!(task_of(&store).fields().claim, claim, "{stop:?}");
    }
    // A kept board's own end wins over what the caller says.
    let (store, _) = stranded();
    let cancelled = swarm(SwarmEnd::Cancelled, [SwarmItemState::Ready; 3]);
    let request = RunEnd {
        stop: Some(StopKind::Blocked),
        ..ending(RunEndOutcome::Stopped)
    };
    ender_keeping(&store, Some(cancelled))
        .execute(&request)
        .unwrap();
    assert_eq!(task_of(&store).fields().status, TaskStatus::Ready);
}

/// #2705 T5 review round 1: the holder renews the task between the end's
/// read and its write; the end loses the race, is built again on the
/// renewed task, and still moves it, keeping the renewed claim.
#[test]
fn an_end_that_loses_the_race_to_a_renewal_still_moves_the_task() {
    use crate::domain::project_board::services::claims::renew;
    let (store, held) = stranded();
    let other = store.client_as(person("Ada"));
    store.before_next_write(move |board| {
        let snapshot = board.read().unwrap();
        let renewed = renew(&snapshot.tasks[0], &person("Ada"), &at("10:20:00")).unwrap();
        let written = other.write(
            &snapshot.head,
            &[BoardArtifact::Task(renewed)],
            "board: renew",
        );
        assert!(matches!(written, Ok(BoardWrite::Written(_))), "{written:?}");
    });
    let done = [SwarmItemState::Completed; 3];
    let mut facts = OpFacts::default();
    ender_keeping(&store, Some(swarm(SwarmEnd::Succeeded, done)))
        .execute_noted(&ending(RunEndOutcome::Succeeded), Some(&mut facts))
        .unwrap();
    assert_eq!(facts.attempts, Some(2), "lost once, then written");
    let task = task_of(&store);
    assert_eq!(task.fields().status, TaskStatus::Review);
    let claim = task.fields().claim.clone().unwrap();
    assert_eq!(claim.since, at("10:20:00"), "built on the renewed task");
    assert_ne!(Some(claim), held.fields().claim);
    assert_eq!(run_1(&store).fields().outcome, Some(RunOutcome::Succeeded));
}
