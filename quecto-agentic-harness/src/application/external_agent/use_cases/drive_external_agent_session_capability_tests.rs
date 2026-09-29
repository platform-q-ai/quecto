//! What the session learns of its agent from `system/init` (#2287 review
//! round 3): whether it names the user turns its results consumed (L1)
//! and whether its interrupt withdraws the queued ones (L3); and the
//! result and interrupt answers the earlier rounds left untested (L4).
//!
//! What claude 2.1.280 says of itself (its bundle, see the PR):
//! - `system/init` carries `claude_code_version` and `capabilities`, "so
//!   SDK consumers can feature-detect instead of version-sniffing";
//! - no capability names `user_message_uuids`, so the version verified to
//!   carry it (2.1.280) is the only word before a result;
//! - `interrupt_cancel_queued_v1`: the interrupt "honors cancel_queued"
//!   (the queued user turns are cancelled and listed on the answer's
//!   `cancelled`); without it "older CLIs ignore the field and behave as
//!   if false": the queued turns survive the interrupt (`still_queued`)
//!   and run afterwards.

use super::test_rig::*;
use super::*;
use crate::application::external_agent::dto::{SessionPhase, SessionRecord};
use crate::domain::external_agent::stream::{ExternalAgentEvent, InterruptReceipt, ResultEvent};

// L1: claude's init arrives as the first turn starts; a CLI whose version
// names turns can be steered in that first turn.
#[tokio::test]
async fn the_first_turn_can_be_steered_once_init_says_the_cli_names_turns() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.feed(capable_init()).await;
    assert_eq!(
        rig.session.steer("two").await,
        Ok(PromptAccepted::Steered { turn: 1 })
    );
    assert_eq!(rig.wire.sent(), ["one", "two"]);
    // Named from init: an id-less success is a turn of claude's own.
    rig.feed(answered(&[], "completed", "claude's own")).await;
    assert_eq!(rig.phase(), SessionPhase::Busy { turn: 1 });
    rig.feed(answered(&["u1", "u2"], "completed", "both")).await;
    assert_eq!(rig.phase(), SessionPhase::Idle);
}

// L1 fallback: an init from an older CLI, or one naming no version, says
// nothing: steers wait, as before, for a result that names its turns.
#[tokio::test]
async fn an_init_without_a_verified_version_teaches_nothing() {
    for version in [Some("2.1.279"), None, Some("unknown")] {
        let rig = started().await;
        rig.session.prompt("one", None).await.unwrap();
        rig.feed(init_event(version, &["interrupt_cancel_queued_v1"]))
            .await;
        assert_eq!(
            rig.session.steer("two").await,
            Err(SessionRefusal::SteerUnavailable),
            "{version:?}"
        );
        rig.feed(answered(&["u1"], "completed", "first")).await;
        rig.session.prompt("three", None).await.unwrap();
        assert_eq!(
            rig.session.steer("four").await,
            Ok(PromptAccepted::Steered { turn: 2 }),
            "learnt from the result: {version:?}"
        );
    }
}

// L3: without `interrupt_cancel_queued_v1` a steer still queued when the
// turn is interrupted survives it and runs afterwards, a turn the session
// could not bound. So no steer is taken from such a CLI: it gets a
// follow-up, which quecto holds and an abort drops.
#[tokio::test]
async fn a_cli_that_cannot_withdraw_a_queued_steer_takes_none() {
    for capabilities in [&["interrupt_receipt_v1"][..], &[]] {
        let rig = started().await;
        rig.session.prompt("one", None).await.unwrap();
        rig.feed(init_event(Some("2.1.280"), capabilities)).await;
        rig.feed(answered(&["u1"], "completed", "first")).await;
        rig.session.prompt("two", None).await.unwrap();
        let refusal = rig.session.steer("three").await.unwrap_err();
        assert_eq!(refusal, SessionRefusal::SteerNotWithdrawable);
        assert_eq!(refusal.kind(), "steer_not_withdrawable");
        assert_eq!(
            refusal.to_string(),
            "the claude-code member cannot steer: its agent does not withdraw a queued steer \
             when a turn is interrupted; send a follow-up"
        );
        assert_eq!(rig.wire.sent(), ["one", "two"], "never written");
        assert_eq!(
            rig.session.follow_up("three").await,
            Ok(PromptAccepted::Queued { position: 1 })
        );
    }
}

// L3: what an init says is kept: a later init without the capabilities
// (claude re-emits init every turn) does not take them back.
#[tokio::test]
async fn a_later_init_does_not_unlearn_the_capabilities() {
    let rig = named_started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.feed(init_event(None, &[])).await;
    assert_eq!(
        rig.session.steer("two").await,
        Ok(PromptAccepted::Steered { turn: 2 })
    );
}

// L4: an id-less result whose `is_error` is absent, from a CLI that names
// turns, is not known to have succeeded: it ends the running turn, as an
// error does.
#[tokio::test]
async fn an_id_less_result_without_is_error_ends_the_running_turn() {
    let rig = named_started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.session.steer("two").await.unwrap();
    let SessionStep::Folded(step) = rig
        .feed(ExternalAgentEvent::Result(ResultEvent {
            is_error: None,
            ..ResultEvent::default()
        }))
        .await
    else {
        panic!("the result is folded")
    };
    assert!(step.turn_end.is_some(), "it ends the turn");
    assert_eq!(rig.phase(), SessionPhase::Idle);
    assert!(
        rig.records
            .all()
            .contains(&SessionRecord::ResultWithoutIds {
                turn: 2,
                ended: true
            })
    );
}

// L4: an interrupt claude refuses (`accepted: false`) withdraws nothing,
// whatever its answer lists: the turn still owes its results, and the
// interrupt's grace still bounds it.
#[tokio::test(start_paused = true)]
async fn a_refused_interrupt_withdraws_nothing() {
    let rig = named_started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.session.steer("two").await.unwrap();
    rig.session.abort().await.unwrap();
    rig.feed(answered(&["u2"], "aborted_tools", "stopped"))
        .await;
    rig.feed(ExternalAgentEvent::InterruptAnswered(InterruptReceipt {
        accepted: false,
        cancelled: vec!["u3".into()],
    }))
    .await;
    assert_eq!(
        rig.phase(),
        SessionPhase::Interrupting { turn: 2 },
        "the steer is still owed"
    );
    assert_eq!(
        rig.paused_step().await,
        Some(SessionStep::Abandoned { turn: 2 })
    );
    assert_eq!(rig.phase(), SessionPhase::Ended);
}

// L4: a refused interrupt followed by the result it owes ends the turn as
// usual.
#[tokio::test]
async fn a_refused_interrupt_then_the_owed_result_ends_the_turn() {
    let rig = named_started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.session.abort().await.unwrap();
    rig.feed(ExternalAgentEvent::InterruptAnswered(
        InterruptReceipt::default(),
    ))
    .await;
    assert_eq!(rig.phase(), SessionPhase::Interrupting { turn: 2 });
    rig.feed(answered(&["u2"], "completed", "finished anyway"))
        .await;
    assert_eq!(rig.phase(), SessionPhase::Idle);
    assert!(!rig.wire.dropped(), "the member lives on");
}
