//! The member session's result binding and its races (#2287 review round
//! 2): how many user turns one turn may hold, results that name none, an
//! older CLI's steers, `close` racing an interrupt, and an overdue
//! interrupt under a busy stream.
//!
//! What claude does (the 2.1.280 bundle, see the PR):
//! - a result's `user_message_uuids` holds at most 64 ids: the collector
//!   keeps the first 64 and overwrites the last slot past that;
//! - the list is absent on "delivery-failure and zeroed results", on
//!   "session-scoped failures with no single triggering send", on turns
//!   that "neither had a client uuid nor folded a user message in" (a turn
//!   of claude's own) and from older producers.

use std::sync::Arc;

use super::test_rig::*;
use super::*;
use crate::application::external_agent::dto::{
    SessionPhase, SessionRecord, USER_TURNS_PER_TURN_CAPACITY,
};

// M1: claude names at most 64 user turns per result, so a turn holding
// more could never be answered in full and the member would stay busy
// for good. The prompt and its steers are capped below that; a steer past
// the cap is refused with quecto's queue-full text, and the result naming
// every accepted one ends the turn.
#[tokio::test]
async fn in_flight_user_turns_are_capped_below_the_clis_uuid_list() {
    assert_eq!(USER_TURNS_PER_TURN_CAPACITY, 63, "one below the CLI's 64");
    let rig = named_started().await;
    rig.session.prompt("prompt", None).await.unwrap();
    let mut accepted = vec!["u2".to_string()];
    let mut refused = 0;
    for n in 0..70 {
        match rig.session.steer(&format!("steer {n}")).await {
            Ok(PromptAccepted::Steered { turn: 2 }) => {
                accepted.push(format!("u{}", rig.wire.sent().len()))
            }
            Err(SessionRefusal::QueueFull) => refused += 1,
            other => panic!("steer {n}: {other:?}"),
        }
    }
    assert_eq!(accepted.len(), USER_TURNS_PER_TURN_CAPACITY);
    assert_eq!(refused, 70 + 1 - USER_TURNS_PER_TURN_CAPACITY);
    assert_eq!(
        SessionRefusal::QueueFull.to_string(),
        "pending prompt queue is full; instruction was not retained"
    );
    assert_eq!(
        rig.wire.sent().len(),
        1 + USER_TURNS_PER_TURN_CAPACITY,
        "a refused steer is never written"
    );
    let ids: Vec<&str> = accepted.iter().map(String::as_str).collect();
    let SessionStep::Folded(step) = rig.feed(answered(&ids, "completed", "all")).await else {
        panic!("the result is folded")
    };
    assert!(
        step.turn_end.is_some(),
        "the result naming them all ends it"
    );
    assert_eq!(rig.phase(), SessionPhase::Idle);
    // The next turn has the whole allowance again.
    assert_eq!(
        rig.session.prompt("next", None).await,
        Ok(PromptAccepted::Started { turn: 3 })
    );
    assert_eq!(
        rig.session.steer("again").await,
        Ok(PromptAccepted::Steered { turn: 3 })
    );
}

// L5: an error result that names no user turn, once claude names them (a
// session-scoped failure, a zeroed or delivery-failure result), ends the
// running turn: none of those is followed by another result for it.
#[tokio::test]
async fn an_id_less_error_result_ends_the_running_turn() {
    let rig = named_started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.session.steer("two").await.unwrap();
    rig.session.follow_up("three").await.unwrap();
    let SessionStep::Folded(step) = rig.feed(result(true, "error_during_execution", None)).await
    else {
        panic!("the result is folded")
    };
    let end = step.turn_end.expect("the id-less error ends the turn");
    assert!(!end.end.is_completed());
    assert!(
        rig.records
            .all()
            .contains(&SessionRecord::ResultWithoutIds {
                turn: 2,
                ended: true
            })
    );
    assert_eq!(rig.phase(), SessionPhase::Busy { turn: 3 }, "the follow-up");
    // A late result naming the ended turn's user turns ends no other.
    rig.feed(answered(&["u2", "u3"], "completed", "late")).await;
    assert_eq!(rig.phase(), SessionPhase::Busy { turn: 3 });
    rig.feed(answered(&["u4"], "completed", "three")).await;
    assert_eq!(rig.phase(), SessionPhase::Idle);
}

// L6: until claude has named the user turns a result answers, a result is
// taken to answer everything written; a steer run as its own turn would
// then have its result taken for the next turn's. So no steer is taken
// until the ids are known; follow-ups are.
#[tokio::test]
async fn an_older_cli_takes_no_steer_until_it_names_the_turns_it_answers() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    // An older version (#2287 review round 3, L1): its init says nothing
    // of naming turns, though it withdraws queued ones on an interrupt.
    rig.feed(init_event(Some("2.1.279"), &["interrupt_cancel_queued_v1"]))
        .await;
    let refusal = rig.session.steer("two").await.unwrap_err();
    assert_eq!(refusal, SessionRefusal::SteerUnavailable);
    assert_eq!(
        refusal.to_string(),
        "the claude-code member cannot steer until its agent names the turns its results \
         answer; send a follow-up"
    );
    assert_eq!(rig.wire.sent(), ["one"], "a refused steer is never written");
    assert!(rig.records.all().contains(&SessionRecord::PromptRefused {
        refusal: "steer_unavailable",
        bytes: 3,
    }));
    rig.session.follow_up("three").await.unwrap();
    rig.feed(completed("first")).await;
    assert_eq!(rig.phase(), SessionPhase::Busy { turn: 2 });
    assert_eq!(
        rig.session.steer("four").await,
        Err(SessionRefusal::SteerUnavailable),
        "a result naming none teaches nothing"
    );
    rig.feed(answered(&["u2"], "completed", "second")).await;
    assert_eq!(rig.phase(), SessionPhase::Idle);
    rig.session.prompt("five", None).await.unwrap();
    assert_eq!(
        rig.session.steer("six").await,
        Ok(PromptAccepted::Steered { turn: 3 }),
        "once named, steers are taken"
    );
}

// M2: `close` while an abort's interrupt is being written ends the member;
// the interrupt's `Ok` then finds it ended, and nothing panics.
#[tokio::test]
async fn close_racing_an_abort_ends_the_member_without_a_panic() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.session.follow_up("two").await.unwrap();
    *rig.wire.close_on_interrupt.lock().unwrap() = Some(Arc::downgrade(&rig.session));
    assert_eq!(
        rig.session.abort().await,
        Ok(AbortOutcome {
            turn: Some(1),
            dropped_follow_ups: 1,
            member_ended: true,
        })
    );
    assert_eq!(rig.phase(), SessionPhase::Ended);
    assert!(rig.wire.dropped());
    let kinds = rig.records.kinds();
    assert!(kinds.contains(&"closed"), "{kinds:?}");
    assert!(!kinds.contains(&"interrupted"), "{kinds:?}");
    assert_eq!(rig.step().await, None);
}

// M2: the same race on a lost turn's interrupt.
#[tokio::test(start_paused = true)]
async fn close_racing_a_lost_turns_interrupt_ends_the_member_without_a_panic() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.feed(skipped(9)).await;
    *rig.wire.close_on_interrupt.lock().unwrap() = Some(Arc::downgrade(&rig.session));
    assert_eq!(rig.paused_step().await, None, "the member has ended");
    assert_eq!(rig.phase(), SessionPhase::Ended);
    assert_eq!(rig.wire.interrupts(), 1);
    assert!(!rig.records.kinds().contains(&"interrupted"));
}

// Nit: an overdue interrupt is acted on before another event is read, so
// a stream that keeps talking cannot starve it.
#[tokio::test(start_paused = true)]
async fn an_overdue_interrupt_is_not_starved_by_a_busy_stream() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.session.abort().await.unwrap();
    tokio::time::advance(INTERRUPT + std::time::Duration::from_secs(1)).await;
    rig.wire.emit(text_block("m1", "still streaming"));
    assert_eq!(rig.step().await, Some(SessionStep::Abandoned { turn: 1 }));
    assert_eq!(rig.phase(), SessionPhase::Ended);
}

// Nit: an abandoned lost turn records how many queued follow-ups the
// member's end dropped.
#[tokio::test(start_paused = true)]
async fn an_abandoned_lost_turn_records_the_follow_ups_it_dropped() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.session.follow_up("two").await.unwrap();
    rig.session.follow_up("three").await.unwrap();
    rig.feed(skipped(9)).await;
    assert_eq!(
        rig.paused_step().await,
        Some(SessionStep::TurnLost { turn: 1 })
    );
    assert_eq!(
        rig.paused_step().await,
        Some(SessionStep::Abandoned { turn: 1 })
    );
    assert!(rig.records.all().contains(&SessionRecord::Abandoned {
        turn: 1,
        dropped_follow_ups: 2,
    }));
}
