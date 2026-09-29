//! #2304 review round 2 (M1, L1, L2): waiting for an ended member's
//! process to exit never holds the write gate and never depends on the
//! caller's future. An agent that does not exit once its input is closed
//! (the rig's `hang_exit`) holds up nobody: an abort answers at once, a
//! prompt racing it is refused at once, and the end is recorded once
//! [`EXIT_GRACE`] has passed, after every other lifecycle record, even when
//! the caller gave up on `close`.

use std::sync::atomic::Ordering;
use std::time::Duration;

use super::test_rig::*;
use crate::application::external_agent::dto::{
    EXIT_GRACE, SessionRecord, SessionRefusal, SessionStep,
};

/// What "at once" means here: well under a second.
const PROMPTLY: Duration = Duration::from_secs(1);

/// A member mid-turn whose process never exits and whose interrupts
/// cannot be written: an abort ends it.
async fn wedged_mid_turn() -> Rig {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.wire.hang_exit.store(true, Ordering::SeqCst);
    rig.wire.refuse_interrupts.store(true, Ordering::SeqCst);
    rig
}

fn ends(rig: &Rig) -> Vec<SessionRecord> {
    rig.records
        .all()
        .into_iter()
        .filter(|record| matches!(record, SessionRecord::Ended { .. }))
        .collect()
}

/// The lifecycle records, in order: what the event log files as
/// `external_agent_lifecycle`.
fn lifecycle(rig: &Rig) -> Vec<&'static str> {
    rig.records
        .kinds()
        .into_iter()
        .filter(|kind| {
            matches!(
                *kind,
                "started" | "interrupted" | "aborted" | "abandoned" | "closed" | "ended"
            )
        })
        .collect()
}

#[tokio::test(start_paused = true)]
async fn an_abort_that_ends_the_member_answers_before_its_process_exits() {
    let rig = wedged_mid_turn().await;
    let outcome = tokio::time::timeout(PROMPTLY, rig.session.abort())
        .await
        .expect("the abort answers at once")
        .expect("a started member can be aborted");
    assert!(outcome.member_ended, "an unwritable interrupt ends it");
}

#[tokio::test(start_paused = true)]
async fn a_prompt_racing_an_ending_abort_is_refused_at_once() {
    let rig = wedged_mid_turn().await;
    let session = rig.session.clone();
    let abort = tokio::spawn(async move { session.abort().await });
    // The abort takes the write gate and ends the member.
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    let refused = tokio::time::timeout(PROMPTLY, rig.session.prompt("two", None))
        .await
        .expect("the prompt is answered at once");
    assert_eq!(refused, Err(SessionRefusal::Ended));
    abort.abort();
}

#[tokio::test(start_paused = true)]
async fn the_end_is_recorded_last_once_the_exit_grace_has_passed() {
    let rig = wedged_mid_turn().await;
    let session = rig.session.clone();
    let abort = tokio::spawn(async move { session.abort().await });
    tokio::time::sleep(EXIT_GRACE - Duration::from_secs(1)).await;
    assert!(ends(&rig).is_empty(), "the process may still exit");
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(
        ends(&rig),
        [SessionRecord::Ended {
            clean: false,
            exit_code: None,
            signal: None,
            wall_ms: Some(EXIT_GRACE.as_millis() as u64),
        }],
        "unobserved, at the grace"
    );
    assert_eq!(
        lifecycle(&rig),
        ["started", "abandoned", "aborted", "ended"],
        "the end is the last lifecycle record"
    );
    abort.abort();
}

#[tokio::test(start_paused = true)]
async fn a_close_its_caller_gave_up_on_still_records_the_end() {
    let rig = started().await;
    rig.wire.hang_exit.store(true, Ordering::SeqCst);
    let closed = tokio::time::timeout(PROMPTLY, rig.session.close()).await;
    assert!(closed.is_err(), "close waits for the end: {closed:?}");
    tokio::time::sleep(EXIT_GRACE).await;
    assert_eq!(ends(&rig).len(), 1, "{:?}", rig.records.kinds());
    assert_eq!(lifecycle(&rig), ["started", "closed", "ended"]);
}

#[tokio::test(start_paused = true)]
async fn an_overdue_interrupt_ends_the_member_without_waiting_for_its_exit() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.session.abort().await.unwrap();
    rig.wire.hang_exit.store(true, Ordering::SeqCst);
    let started = tokio::time::Instant::now();
    assert_eq!(
        rig.paused_step().await,
        Some(SessionStep::Abandoned { turn: 1 })
    );
    assert!(
        started.elapsed() < INTERRUPT + PROMPTLY,
        "the reader is told at the interrupt's deadline: {:?}",
        started.elapsed()
    );
    let refused = tokio::time::timeout(PROMPTLY, rig.session.prompt("two", None))
        .await
        .expect("the prompt is answered at once");
    assert_eq!(refused, Err(SessionRefusal::Ended));
    tokio::time::sleep(EXIT_GRACE + Duration::from_millis(1)).await;
    assert_eq!(ends(&rig).len(), 1, "{:?}", rig.records.kinds());
}
