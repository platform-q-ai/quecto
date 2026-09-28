//! The member session's interrupts and result binding (#2287 review round
//! 1): a lost turn, an abort, a steer racing the turn's end and a failed
//! follow-up write.
//!
//! What claude does (spike #2264 and the 2.1.280 CLI, see the PR):
//! - it emits exactly one `result` per turn, naming every user turn it
//!   consumed (`user_message_uuids`): a turn written mid-turn is folded
//!   into the running turn, or runs as its own turn (with its own
//!   `result`) when the running one has already ended;
//! - an interrupt (`control_request` `interrupt`, `cancel_queued`) stops
//!   the running turn, which still ends with its one `result`, withdraws
//!   the queued user turns (named in its `control_response`) and leaves
//!   the process taking turns.
//!
//! quecto's own abort (`uds_dispatch.rs::handle_abort`) discards pending
//! work, answers Ok and keeps the agent: so does the member's.

use super::test_rig::*;
use super::*;
use crate::application::external_agent::dto::{SessionPhase, SessionRecord};

// H1: the skipped-line grace covers only the silence right after a skip:
// any later event disarms it, and a long quiet tool call after that never
// ends the turn.
#[tokio::test(start_paused = true)]
async fn an_event_after_a_skipped_line_disarms_the_grace() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.feed(skipped(40)).await;
    rig.feed(text_block("m1", "still here")).await;
    rig.stays_quiet(GRACE + std::time::Duration::from_secs(1))
        .await;
    assert_eq!(rig.phase(), SessionPhase::Busy { turn: 1 });
    assert_eq!(rig.wire.interrupts(), 0, "nothing is interrupted");
}

// H2 (the reviewer's scenario): prompt one, follow_up two, a skipped line,
// 60 s quiet, then the late result for one. The lost turn is interrupted;
// nothing is written until the result it owes comes, which is then never
// taken for turn two's.
#[tokio::test(start_paused = true)]
async fn a_lost_turn_is_interrupted_and_its_late_result_is_never_misattributed() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.session.follow_up("two").await.unwrap();
    rig.feed(skipped(17 * 1024 * 1024)).await;
    let started = tokio::time::Instant::now();
    assert_eq!(
        rig.paused_step().await,
        Some(SessionStep::TurnLost { turn: 1 })
    );
    assert!(started.elapsed() >= GRACE, "only after the grace period");
    assert_eq!(rig.wire.interrupts(), 1, "the lost turn is interrupted");
    assert_eq!(rig.phase(), SessionPhase::Interrupting { turn: 1 });
    assert_eq!(rig.wire.sent(), ["one"], "nothing new is written");
    // claude's state is unknown: no prompt is accepted meanwhile.
    assert_eq!(
        rig.session.prompt("three", None).await,
        Err(SessionRefusal::Busy)
    );
    assert_eq!(
        rig.session.steer("three").await,
        Err(SessionRefusal::Interrupting)
    );
    assert_eq!(rig.wire.sent(), ["one"]);

    rig.feed(interrupt_answered(&[])).await;
    assert_eq!(rig.phase(), SessionPhase::Interrupting { turn: 1 });
    let SessionStep::Folded(step) = rig.feed(answered(&["u1"], "aborted_tools", "late")).await
    else {
        panic!("the late result is folded")
    };
    assert!(step.turn_end.is_some(), "it ends the lost turn, turn 1");
    assert_eq!(rig.wire.sent(), ["one", "two"], "then the follow-up starts");
    assert_eq!(rig.phase(), SessionPhase::Busy { turn: 2 });
    rig.feed(answered(&["u2"], "completed", "second")).await;
    assert_eq!(rig.phase(), SessionPhase::Idle);
    assert_eq!(rig.session.report().unwrap().content, "second");
}

// H2: a lost turn whose result never comes (the skipped line was it)
// leaves claude's state unknown: the member is ended, not reused.
#[tokio::test(start_paused = true)]
async fn an_interrupted_turn_that_never_answers_ends_the_member() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.feed(skipped(9)).await;
    assert_eq!(
        rig.paused_step().await,
        Some(SessionStep::TurnLost { turn: 1 })
    );
    let started = tokio::time::Instant::now();
    rig.feed(interrupt_answered(&[])).await;
    assert_eq!(
        rig.paused_step().await,
        Some(SessionStep::Abandoned { turn: 1 })
    );
    // The grace runs from the interrupt, which the reply did not reset.
    assert!(
        started.elapsed() <= INTERRUPT,
        "bounded by the interrupt grace: {:?}",
        started.elapsed()
    );
    assert_eq!(rig.phase(), SessionPhase::Ended);
    assert!(rig.wire.dropped(), "the agent process is ended");
    assert_eq!(rig.wire.sent(), ["one"]);
    assert_eq!(rig.session.next_step().await, None);
    assert!(rig.records.all().contains(&SessionRecord::Abandoned {
        turn: 1,
        dropped_follow_ups: 0
    }));
}

// M1: quecto's idle abort discards pending work, answers Ok and keeps the
// agent (`handle_abort`).
#[tokio::test]
async fn an_idle_abort_answers_ok_and_keeps_the_member() {
    let rig = started().await;
    assert_eq!(
        rig.session.abort().await,
        Ok(AbortOutcome {
            turn: None,
            dropped_follow_ups: 0,
            member_ended: false,
        })
    );
    assert!(!rig.wire.dropped(), "the agent process lives on");
    assert_eq!(rig.wire.interrupts(), 0, "nothing runs to interrupt");
    assert_eq!(rig.phase(), SessionPhase::Idle);
    assert_eq!(
        rig.session.prompt("next", None).await,
        Ok(PromptAccepted::Started { turn: 1 })
    );
}

// M1: a busy abort interrupts the turn and discards the follow-ups; the
// session stays busy until the stopped turn's result, then the member
// takes prompts again.
#[tokio::test]
async fn a_busy_abort_interrupts_the_turn_and_the_member_survives() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.session.follow_up("two").await.unwrap();
    assert_eq!(
        rig.session.abort().await,
        Ok(AbortOutcome {
            turn: Some(1),
            dropped_follow_ups: 1,
            member_ended: false,
        })
    );
    assert_eq!(rig.wire.interrupts(), 1);
    assert_eq!(rig.phase(), SessionPhase::Interrupting { turn: 1 });
    assert_eq!(rig.session.state().queued_follow_ups, 0);
    // A second abort while interrupting sends nothing more.
    assert_eq!(
        rig.session.abort().await,
        Ok(AbortOutcome {
            turn: Some(1),
            dropped_follow_ups: 0,
            member_ended: false,
        })
    );
    assert_eq!(rig.wire.interrupts(), 1);
    rig.feed(interrupt_answered(&[])).await;
    let SessionStep::Folded(step) = rig
        .feed(answered(&["u1"], "aborted_streaming", "stopped"))
        .await
    else {
        panic!("the stopped turn's result is folded")
    };
    assert!(step.turn_end.is_some());
    assert_eq!(rig.phase(), SessionPhase::Idle);
    assert!(!rig.wire.dropped());
    assert_eq!(rig.wire.sent(), ["one"], "the follow-up was discarded");
    assert_eq!(
        rig.session.prompt("again", None).await,
        Ok(PromptAccepted::Started { turn: 2 })
    );
    let kinds = rig.records.kinds();
    assert!(
        kinds.ends_with(&[
            "interrupted",
            "aborted",
            "aborted",
            "turn_ended",
            "prompt_accepted"
        ]),
        "{kinds:?}"
    );
}

// M1: a steer still queued in claude is withdrawn by the interrupt, named
// in its answer: no result is owed for it.
#[tokio::test]
async fn an_interrupt_that_withdraws_a_queued_steer_owes_no_result_for_it() {
    let rig = named_started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.session.steer("two").await.unwrap();
    rig.session.abort().await.unwrap();
    rig.feed(answered(&["u2"], "aborted_tools", "stopped"))
        .await;
    assert_eq!(
        rig.phase(),
        SessionPhase::Interrupting { turn: 2 },
        "the steer is still owed"
    );
    let SessionStep::Folded(step) = rig.feed(interrupt_answered(&["u3"])).await else {
        panic!("the answer is folded")
    };
    let end = step
        .turn_end
        .expect("the turn ends with its stopped result's outcome");
    assert!(!end.end.is_completed());
    assert_eq!(rig.phase(), SessionPhase::Idle);
    assert!(rig.records.all().contains(&SessionRecord::TurnEnded {
        turn: 2,
        outcome: "aborted",
        duration_ms: Some(1200),
        cost_micro_usd: 0,
    }));
}

// M1: a busy abort whose turn never answers ends the member.
#[tokio::test(start_paused = true)]
async fn a_busy_abort_whose_turn_never_answers_ends_the_member() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.session.abort().await.unwrap();
    let started = tokio::time::Instant::now();
    assert_eq!(
        rig.paused_step().await,
        Some(SessionStep::Abandoned { turn: 1 })
    );
    assert!(started.elapsed() >= INTERRUPT, "only after the grace");
    assert_eq!(rig.phase(), SessionPhase::Ended);
    assert!(rig.wire.dropped());
}

// M1: an interrupt that cannot be written leaves claude's state unknown:
// the abort ends the member.
#[tokio::test]
async fn an_abort_whose_interrupt_cannot_be_written_ends_the_member() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.wire
        .refuse_interrupts
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        rig.session.abort().await,
        Ok(AbortOutcome {
            turn: Some(1),
            dropped_follow_ups: 0,
            member_ended: true,
        })
    );
    assert_eq!(rig.phase(), SessionPhase::Ended);
    assert!(rig.wire.dropped());
    assert_eq!(rig.session.next_step().await, None);
}

// M2 (the reviewer's scenario): claude emitted turn one's result; a steer
// is written before the session folds it, so claude runs it as a turn of
// its own. The session stays busy until that turn's result.
#[tokio::test]
async fn a_steer_racing_the_result_keeps_the_session_busy_until_its_own_result() {
    let rig = named_started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.wire.emit(answered(&["u2"], "completed", "first"));
    assert_eq!(
        rig.session.steer("two").await,
        Ok(PromptAccepted::Steered { turn: 2 })
    );
    let SessionStep::Folded(step) = rig.step().await.unwrap() else {
        panic!("the first result is folded")
    };
    assert_eq!(step.turn_end, None, "the turn goes on: the steer is owed");
    assert_eq!(rig.phase(), SessionPhase::Busy { turn: 2 });
    assert_eq!(
        rig.session.prompt("three", None).await,
        Err(SessionRefusal::Busy),
        "never accepted while claude runs the steer"
    );
    assert!(
        rig.records
            .all()
            .contains(&SessionRecord::TurnContinued { turn: 2, owed: 1 })
    );
    let SessionStep::Folded(step) = rig.feed(answered(&["u3"], "completed", "second")).await else {
        panic!("the steer's result is folded")
    };
    assert!(step.turn_end.is_some());
    assert_eq!(rig.phase(), SessionPhase::Idle);
    assert_eq!(
        rig.session.state().totals.turns,
        3,
        "claude ran two turns after the first"
    );
}

// Once claude names the turns it consumed, a result naming none of ours
// (a turn claude started by itself) ends none of the session's turns.
#[tokio::test]
async fn a_result_naming_no_turn_of_ours_ends_no_turn() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.feed(answered(&["u1"], "completed", "first")).await;
    rig.session.prompt("two", None).await.unwrap();
    rig.feed(answered(&[], "completed", "a task notification's turn"))
        .await;
    assert_eq!(rig.phase(), SessionPhase::Busy { turn: 2 });
    // Recorded (#2287 review 2, L5): a success naming no user turn is a
    // turn of claude's own, which consumed none of the member's.
    assert!(
        rig.records
            .all()
            .contains(&SessionRecord::ResultWithoutIds {
                turn: 2,
                ended: false
            })
    );
    rig.feed(answered(&["u2"], "completed", "second")).await;
    assert_eq!(rig.phase(), SessionPhase::Idle);
}

// L4: a follow-up that cannot be written is recorded and surfaced to the
// reader, as quecto marks a failed queued prompt's control receipt.
#[tokio::test]
async fn a_failed_follow_up_write_is_recorded_and_surfaced() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.session.follow_up("two").await.unwrap();
    rig.wire
        .closed
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let SessionStep::Folded(step) = rig.feed(answered(&["u1"], "completed", "first")).await else {
        panic!("the result is folded")
    };
    assert!(step.turn_end.is_some());
    assert_eq!(
        rig.step().await,
        Some(SessionStep::FollowUpFailed {
            turn: 2,
            refusal: SessionRefusal::Input(
                crate::application::external_agent::dto::ExternalAgentInputError::Closed
            ),
        })
    );
    assert_eq!(rig.phase(), SessionPhase::Idle);
    assert!(rig.records.all().contains(&SessionRecord::FollowUpFailed {
        turn: 2,
        bytes: 3,
        refusal: "input",
    }));
}

// The CLI's stub runner (and S4's teardown) ends the member with `close`.
#[tokio::test]
async fn close_ends_the_member() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.session.follow_up("two").await.unwrap();
    assert_eq!(
        rig.session.close().await,
        Ok(AbortOutcome {
            turn: Some(1),
            dropped_follow_ups: 1,
            member_ended: true,
        })
    );
    assert!(rig.wire.dropped());
    assert_eq!(rig.phase(), SessionPhase::Ended);
    assert_eq!(rig.session.close().await, Err(SessionRefusal::Ended));
    assert_eq!(rig.records.kinds().last(), Some(&"closed"));
}
