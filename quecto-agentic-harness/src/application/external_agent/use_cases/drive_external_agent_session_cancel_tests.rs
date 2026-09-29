//! A caller's future dropped mid-way loses nothing (#2287 review round 5):
//! an event the reader has read is folded even if the reader is cancelled
//! while it waits for the write gate, and a user turn is either not
//! queued at all (the session is as it was) or queued and owed a result,
//! whichever await the prompt, steer or follow-up is cancelled at.

use std::sync::atomic::Ordering;
use std::time::Duration;

use super::test_rig::*;
use super::*;
use crate::application::external_agent::dto::SessionPhase;

/// Long enough for every task that can run to run (paused time).
const CANCEL_AFTER: Duration = Duration::from_secs(1);

/// Let every spawned task that can run, run until it waits.
async fn run_ready_tasks() {
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
}

// The reviewer's scenario: a steer holds the write gate (its write is not
// yet acknowledged) when the running turn's result is read. The reader,
// waiting for the gate, is cancelled: the result it read is still folded
// by the next step, and the turn ends once the steer is answered too.
#[tokio::test(start_paused = true)]
async fn a_reader_cancelled_at_the_write_gate_keeps_the_event_it_read() {
    let rig = named_started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.wire.hold_ack(true);
    let session = rig.session.clone();
    let steer = tokio::spawn(async move { session.steer("two").await });
    run_ready_tasks().await;
    assert!(!steer.is_finished(), "the steer holds the write gate");
    rig.wire.emit(answered(&["u2"], "completed", "one"));
    let cancelled = tokio::time::timeout(CANCEL_AFTER, rig.session.next_step()).await;
    assert!(cancelled.is_err(), "the reader waits for the gate");
    rig.wire.hold_ack(false);
    assert_eq!(
        steer.await.unwrap(),
        Ok(PromptAccepted::Steered { turn: 2 })
    );
    assert!(matches!(rig.step().await, Some(SessionStep::Folded(_))));
    assert_eq!(
        rig.phase(),
        SessionPhase::Busy { turn: 2 },
        "u2's result was folded; the steer (u3) is still owed"
    );
    rig.feed(answered(&["u3"], "completed", "two")).await;
    assert_eq!(rig.phase(), SessionPhase::Idle);
}

// Cancelled before its user turn is queued, a prompt leaves nothing
// behind: nothing is written and the member is idle (the turn ordinal it
// took is not reused).
#[tokio::test(start_paused = true)]
async fn a_prompt_cancelled_before_it_is_queued_leaves_the_member_idle() {
    let rig = started().await;
    rig.wire.hold_queue(true);
    let cancelled = tokio::time::timeout(CANCEL_AFTER, rig.session.prompt("one", None)).await;
    assert!(cancelled.is_err(), "the prompt waits to be queued");
    assert!(rig.wire.sent().is_empty(), "nothing is written");
    assert_eq!(rig.phase(), SessionPhase::Idle);
    rig.wire.hold_queue(false);
    assert_eq!(
        rig.session.prompt("two", None).await,
        Ok(PromptAccepted::Started { turn: 2 })
    );
    assert_eq!(rig.wire.sent(), ["two"]);
}

// Cancelled once its user turn is queued, a prompt's turn runs: the turn
// is recorded, owed its result, and ends on it.
#[tokio::test(start_paused = true)]
async fn a_prompt_cancelled_after_it_is_queued_runs_its_turn() {
    let rig = named_started().await;
    rig.wire.hold_ack(true);
    let cancelled = tokio::time::timeout(CANCEL_AFTER, rig.session.prompt("one", None)).await;
    assert!(cancelled.is_err(), "the prompt waits for its write");
    assert_eq!(rig.wire.sent(), ["zero", "one"]);
    assert_eq!(rig.phase(), SessionPhase::Busy { turn: 2 });
    assert_eq!(user_messages(&rig.session), ["zero", "one"]);
    rig.feed(answered(&["u2"], "completed", "one")).await;
    assert_eq!(rig.phase(), SessionPhase::Idle);
}

// Cancelled once its user turn is queued, a steer is owed a result: the
// running turn's own result does not end the turn.
#[tokio::test(start_paused = true)]
async fn a_steer_cancelled_after_it_is_queued_is_still_owed() {
    let rig = named_started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.wire.hold_ack(true);
    let cancelled = tokio::time::timeout(CANCEL_AFTER, rig.session.steer("two")).await;
    assert!(cancelled.is_err(), "the steer waits for its write");
    assert_eq!(rig.wire.sent(), ["zero", "one", "two"]);
    rig.wire.hold_ack(false);
    rig.feed(answered(&["u2"], "completed", "one")).await;
    assert_eq!(
        rig.phase(),
        SessionPhase::Busy { turn: 2 },
        "the steer (u3) is still owed"
    );
    assert_eq!(user_messages(&rig.session), ["zero", "one", "two"]);
    rig.feed(answered(&["u3"], "completed", "two")).await;
    assert_eq!(rig.phase(), SessionPhase::Idle);
}

// The reader is cancelled while it starts a follow-up whose user turn
// cannot be queued yet: the follow-up is not lost, it is written before
// anything else, and its turn runs.
#[tokio::test(start_paused = true)]
async fn a_reader_cancelled_while_starting_a_follow_up_still_starts_it() {
    let rig = named_started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.session.follow_up("two").await.unwrap();
    rig.wire.hold_queue(true);
    rig.wire.emit(answered(&["u2"], "completed", "one"));
    let cancelled = tokio::time::timeout(CANCEL_AFTER, rig.session.next_step()).await;
    assert!(
        cancelled.is_err(),
        "the reader waits to queue the follow-up"
    );
    assert_eq!(rig.wire.sent(), ["zero", "one"]);
    rig.wire.hold_queue(false);
    assert!(matches!(rig.step().await, Some(SessionStep::Folded(_))));
    assert_eq!(rig.wire.sent(), ["zero", "one", "two"]);
    assert_eq!(rig.phase(), SessionPhase::Busy { turn: 3 });
    rig.feed(answered(&["u3"], "completed", "two")).await;
    assert_eq!(rig.phase(), SessionPhase::Idle);
}

// A user turn whose write provably failed once queued (its writer
// failed) is rolled back: nothing owes it, a turn it began never started,
// and a steer's turn ends on the result of what was written.
#[tokio::test(start_paused = true)]
async fn a_queued_user_turn_whose_write_fails_is_rolled_back() {
    let rig = named_started().await;
    rig.wire.fail_ack.store(true, Ordering::SeqCst);
    assert!(matches!(
        rig.session.prompt("one", None).await,
        Err(SessionRefusal::Input(_))
    ));
    assert_eq!(rig.phase(), SessionPhase::Idle);
    rig.wire.fail_ack.store(false, Ordering::SeqCst);
    assert_eq!(
        rig.session.prompt("two", None).await,
        Ok(PromptAccepted::Started { turn: 3 })
    );
    rig.wire.fail_ack.store(true, Ordering::SeqCst);
    assert!(matches!(
        rig.session.steer("three").await,
        Err(SessionRefusal::Input(_))
    ));
    assert_eq!(rig.phase(), SessionPhase::Busy { turn: 3 });
    rig.feed(answered(&["u3"], "completed", "two")).await;
    assert_eq!(rig.phase(), SessionPhase::Idle, "u4 was never owed");
}
