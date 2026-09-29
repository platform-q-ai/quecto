//! The member session's edges the mutation run found unpinned (#2287): an
//! interrupt answer no interrupt of this turn asked for, a steer whose
//! write failed under a CLI that names no turns, and a grace too long for
//! milliseconds.

use std::sync::atomic::Ordering;
use std::time::Duration;

use super::test_rig::*;
use super::*;
use crate::application::external_agent::dto::SessionPhase;
use crate::domain::external_agent::stream::InterruptReceipt;

// An accepted interrupt answer while the turn runs uninterrupted (a late
// answer to an earlier turn's interrupt) is not this turn's: the turn
// goes on, and ends on its own result.
#[tokio::test]
async fn an_accepted_answer_to_no_interrupt_of_the_turn_does_not_end_it() {
    let rig = named_started().await;
    rig.session.prompt("one", None).await.unwrap();
    let before = rig.records.kinds().len();
    let SessionStep::Folded(step) = rig.feed(interrupt_answered(&["u2"])).await else {
        panic!("the answer is folded")
    };
    assert_eq!(step.turn_end, None);
    assert_eq!(rig.phase(), SessionPhase::Busy { turn: 2 });
    assert!(!rig.records.kinds()[before..].contains(&"turn_ended"));
    let SessionStep::Folded(step) = rig.feed(answered(&["u2"], "completed", "one")).await else {
        panic!("the result is folded")
    };
    assert!(step.turn_end.is_some(), "its own result ends it");
    assert_eq!(rig.phase(), SessionPhase::Idle);
}

// A refused interrupt answer while the turn runs uninterrupted (a late
// refusal of an earlier turn's interrupt) leaves the member alone: only
// a refusal while this turn is interrupted ends it.
#[tokio::test]
async fn a_refused_answer_to_no_interrupt_of_the_turn_leaves_the_member() {
    let rig = named_started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.feed(ExternalAgentEvent::InterruptAnswered(InterruptReceipt {
        accepted: false,
        cancelled: Vec::new(),
    }))
    .await;
    assert_eq!(rig.phase(), SessionPhase::Busy { turn: 2 });
    assert!(!rig.wire.dropped(), "the member lives on");
    assert!(!rig.records.kinds().contains(&"abandoned"));
    rig.feed(answered(&["u2"], "completed", "one")).await;
    assert_eq!(rig.phase(), SessionPhase::Idle);
}

// A steer whose write failed is not in the turn: under a CLI whose init
// admits steers but which names no turns, the turn's id-less success then
// answers the prompt alone and ends the turn, rather than leaving it
// unknown whether it answered a steer.
#[tokio::test]
async fn a_steer_whose_write_failed_is_not_counted_in_the_turn() {
    let rig = started().await;
    rig.feed(capable_init()).await;
    rig.session.prompt("one", None).await.unwrap();
    rig.wire.fail_ack.store(true, Ordering::SeqCst);
    assert!(matches!(
        rig.session.steer("two").await,
        Err(SessionRefusal::Input(_))
    ));
    rig.wire.fail_ack.store(false, Ordering::SeqCst);
    let SessionStep::Folded(step) = rig.feed(completed("one")).await else {
        panic!("the result is folded")
    };
    assert!(step.turn_end.is_some(), "it answered the prompt");
    assert_eq!(rig.phase(), SessionPhase::Idle);
    assert!(!rig.wire.dropped(), "the member lives on");
    assert_eq!(
        rig.session.prompt("three", None).await,
        Ok(PromptAccepted::Started { turn: 2 })
    );
}

// A grace past u64 milliseconds never runs out: it saturates, never
// wraps to nothing.
#[tokio::test(start_paused = true)]
async fn a_grace_too_long_for_milliseconds_never_runs_out() {
    let rig = rig_over(
        false,
        ExternalAgentSessionSettings {
            skipped_line_grace: Duration::MAX,
            ..settings()
        },
    );
    rig.session.start().await.unwrap();
    rig.session.prompt("one", None).await.unwrap();
    rig.feed(skipped(9)).await;
    rig.stays_quiet(Duration::from_secs(24 * 60 * 60)).await;
    assert_eq!(rig.phase(), SessionPhase::Busy { turn: 1 });
    assert_eq!(rig.wire.interrupts(), 0, "nothing is interrupted");
}
