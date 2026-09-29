//! The reader's wait follows the session (#2287 review round 3, M1): an
//! abort or a `close` that changes what the reader waits for wakes a
//! reader already blocked on the stream, so an interrupted turn that
//! never answers ends the member on time even when claude is wedged and
//! writes nothing more.

use std::time::Duration;

use super::test_rig::*;
use super::*;
use crate::application::external_agent::dto::{SessionPhase, SessionRecord};

/// A reader of `rig`'s session, left blocked on the stream: the paused
/// runtime runs it until it waits.
async fn blocked_reader(rig: &Rig) -> tokio::task::JoinHandle<Option<SessionStep>> {
    let session = rig.session.clone();
    let reader = tokio::spawn(async move { session.next_step().await });
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert!(!reader.is_finished(), "the reader waits on the stream");
    reader
}

/// What `reader` returned, within ten minutes of paused time.
async fn joined(reader: tokio::task::JoinHandle<Option<SessionStep>>) -> Option<SessionStep> {
    tokio::time::timeout(Duration::from_secs(600), reader)
        .await
        .expect("the reader returns")
        .expect("the reader does not panic")
}

// The reviewer's scenario: the reader is already waiting for the next
// event when the turn is aborted. claude is wedged and never answers: the
// member is ended once the interrupt's grace has run, not never.
#[tokio::test(start_paused = true)]
async fn an_abort_wakes_a_blocked_reader_to_the_interrupt_deadline() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    let reader = blocked_reader(&rig).await;
    let aborted = tokio::time::Instant::now();
    assert!(!rig.session.abort().await.unwrap().member_ended);
    assert_eq!(rig.phase(), SessionPhase::Interrupting { turn: 1 });
    assert_eq!(
        joined(reader).await,
        Some(SessionStep::Abandoned { turn: 1 })
    );
    let waited = aborted.elapsed();
    assert!(
        waited >= INTERRUPT && waited < INTERRUPT + Duration::from_secs(1),
        "the interrupt's grace, from the abort: {waited:?}"
    );
    assert_eq!(rig.phase(), SessionPhase::Ended);
    assert!(rig.wire.dropped(), "the agent process is ended");
    assert!(rig.records.all().contains(&SessionRecord::Abandoned {
        turn: 1,
        dropped_follow_ups: 0
    }));
}

// The reader is timing a skipped line's grace (60 s) when the turn is
// aborted: the member is ended 30 s after the abort (the interrupt's
// grace), not when the skipped-line grace would have run out.
#[tokio::test(start_paused = true)]
async fn an_abort_during_a_skipped_line_grace_is_timed_from_the_abort() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.feed(skipped(9)).await;
    let reader = blocked_reader(&rig).await;
    tokio::time::advance(Duration::from_secs(10)).await;
    let aborted = tokio::time::Instant::now();
    assert!(!rig.session.abort().await.unwrap().member_ended);
    assert_eq!(
        joined(reader).await,
        Some(SessionStep::Abandoned { turn: 1 })
    );
    let waited = aborted.elapsed();
    assert!(
        waited >= INTERRUPT && waited < INTERRUPT + Duration::from_secs(1),
        "30 s after the abort, not at the skipped-line grace's end: {waited:?}"
    );
    assert_eq!(rig.wire.interrupts(), 1, "the abort's interrupt only");
    assert_eq!(rig.phase(), SessionPhase::Ended);
}

// A skipped line's grace runs from the skipped line, not from whenever the
// reader last started waiting: a reader woken meanwhile gives the turn up
// on time.
#[tokio::test(start_paused = true)]
async fn a_skipped_line_grace_runs_from_the_skipped_line() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.feed(skipped(9)).await;
    let skipped_at = tokio::time::Instant::now();
    let reader = blocked_reader(&rig).await;
    tokio::time::advance(Duration::from_secs(20)).await;
    // A follow-up changes nothing the reader waits for.
    rig.session.follow_up("two").await.unwrap();
    assert_eq!(
        joined(reader).await,
        Some(SessionStep::TurnLost { turn: 1 })
    );
    let waited = skipped_at.elapsed();
    assert!(
        waited >= GRACE && waited < GRACE + Duration::from_secs(1),
        "the grace, from the skipped line: {waited:?}"
    );
}

// `close` while the reader is blocked: the reader returns at once, the
// member ended.
#[tokio::test(start_paused = true)]
async fn close_releases_a_blocked_reader_at_once() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    let reader = blocked_reader(&rig).await;
    let closed = tokio::time::Instant::now();
    rig.session.close().await.unwrap();
    assert_eq!(joined(reader).await, None, "the member has ended");
    assert_eq!(closed.elapsed(), Duration::ZERO, "at once");
    assert_eq!(rig.phase(), SessionPhase::Ended);
}
