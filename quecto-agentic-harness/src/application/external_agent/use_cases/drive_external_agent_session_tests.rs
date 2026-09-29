//! The member session state machine (#2287), over fakes of the S2 ports.
//!
//! Busy semantics mirror quecto's own UDS agent (`protocol_commands.rs`,
//! `uds_prompt_admission.rs::handle_busy_prompt`): a prompt while a turn
//! runs is refused unless it names a `streamingBehavior`; `steer` is
//! delivered into the running turn, `follow_up` once the turn ends.

use super::test_rig::*;
use super::*;
use crate::application::external_agent::dto::{
    ExecutionState, ExternalAgentExit, FOLLOW_UP_QUEUE_CAPACITY, SessionPhase, SessionRecord,
};
use crate::domain::external_agent::stream::{
    AssistantContent, ExternalAgentEvent, SkippedLine, SkippedLineReason,
};

// quecto: `prompt` while running without `streamingBehavior` is rejected
// with "agent is running; provide streamingBehavior" (handle_busy_prompt).
#[tokio::test]
async fn a_prompt_while_busy_is_refused_without_steer_or_follow_up() {
    let rig = started().await;
    assert_eq!(rig.phase(), SessionPhase::Idle);
    assert_eq!(
        rig.session.prompt("one", None).await,
        Ok(PromptAccepted::Started { turn: 1 })
    );
    assert_eq!(rig.phase(), SessionPhase::Busy { turn: 1 });
    let refusal = rig.session.prompt("two", None).await.unwrap_err();
    assert_eq!(refusal, SessionRefusal::Busy);
    assert_eq!(
        refusal.to_string(),
        "agent is running; provide streamingBehavior"
    );
    assert_eq!(
        rig.wire.sent(),
        ["one"],
        "a refused prompt is never written"
    );
    assert_eq!(rig.phase(), SessionPhase::Busy { turn: 1 });
}

// quecto: `steer` interrupts after the current tool and is delivered next;
// a claude process reads stdin mid-turn and folds the text into the
// running turn (spike #2264), so it is written at once and the turn's one
// `result` ends both.
#[tokio::test]
async fn steer_writes_immediately_and_folds_into_the_running_turn() {
    let rig = named_started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.feed(text_block("m1", "working")).await;
    assert_eq!(
        rig.session.steer("two").await,
        Ok(PromptAccepted::Steered { turn: 2 })
    );
    assert_eq!(rig.wire.sent(), ["zero", "one", "two"], "written at once");
    assert_eq!(rig.phase(), SessionPhase::Busy { turn: 2 });
    let SessionStep::Folded(step) = rig.feed(answered(&["u2", "u3"], "completed", "done")).await
    else {
        panic!("the result is folded")
    };
    assert!(step.turn_end.is_some(), "one result ends the steered turn");
    assert_eq!(rig.phase(), SessionPhase::Idle);
    assert_eq!(rig.session.state().totals.turns, 2);
    assert_eq!(user_messages(&rig.session), ["zero", "one", "two"]);
    assert_eq!(rig.session.report().unwrap().content, "done");
}

// quecto: `follow_up` is delivered when the agent finishes.
#[tokio::test]
async fn follow_up_waits_for_turn_end() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    assert_eq!(
        rig.session.follow_up("two").await,
        Ok(PromptAccepted::Queued { position: 1 })
    );
    assert_eq!(
        rig.session
            .prompt("three", Some(StreamingBehavior::FollowUp))
            .await,
        Ok(PromptAccepted::Queued { position: 2 })
    );
    assert_eq!(rig.wire.sent(), ["one"], "follow-ups wait");
    assert_eq!(rig.session.state().queued_follow_ups, 2);
    rig.feed(text_block("m1", "still working")).await;
    assert_eq!(rig.wire.sent(), ["one"], "still waiting mid-turn");

    rig.feed(completed("first")).await;
    assert_eq!(rig.wire.sent(), ["one", "two"], "sent at the turn's end");
    assert_eq!(rig.phase(), SessionPhase::Busy { turn: 2 });
    assert_eq!(rig.session.state().queued_follow_ups, 1);

    rig.feed(completed("second")).await;
    assert_eq!(rig.wire.sent(), ["one", "two", "three"]);
    assert_eq!(rig.phase(), SessionPhase::Busy { turn: 3 });
    rig.feed(completed("third")).await;
    assert_eq!(rig.phase(), SessionPhase::Idle);
    assert_eq!(rig.session.state().totals.turns, 3);
}

#[tokio::test]
async fn queue_overflow_is_refused() {
    let rig = named_started().await;
    rig.session.prompt("one", None).await.unwrap();
    for position in 1..=FOLLOW_UP_QUEUE_CAPACITY {
        assert_eq!(
            rig.session.follow_up(&format!("f{position}")).await,
            Ok(PromptAccepted::Queued { position })
        );
    }
    let refusal = rig.session.follow_up("one too many").await.unwrap_err();
    assert_eq!(refusal, SessionRefusal::QueueFull);
    // quecto's own text and bound (`uds_pending.rs`, `MAX_PENDING`).
    assert_eq!(FOLLOW_UP_QUEUE_CAPACITY, 64);
    assert_eq!(
        refusal.to_string(),
        "pending prompt queue is full; instruction was not retained"
    );
    assert_eq!(
        rig.session.state().queued_follow_ups,
        FOLLOW_UP_QUEUE_CAPACITY
    );
    // A steer is written at once, so a full queue does not refuse it.
    assert_eq!(
        rig.session.steer("steer").await,
        Ok(PromptAccepted::Steered { turn: 2 })
    );
}

#[tokio::test]
async fn a_failed_turn_returns_to_idle_and_reports_the_failure() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    let SessionStep::Folded(step) = rig.feed(result(true, "api_error", None)).await else {
        panic!("the result is folded")
    };
    let outcome = step.turn_end.expect("the failed result ends the turn");
    assert!(!outcome.end.is_completed());
    assert_eq!(rig.phase(), SessionPhase::Idle);
    assert_eq!(rig.session.state().execution, ExecutionState::Idle);
    let report = rig.session.report().expect("a failed turn reports");
    assert!(report.failure.is_some(), "{report:?}");
    assert_eq!(
        report.content,
        "the turn failed (terminal_reason: api_error)"
    );
    // The member takes the next prompt.
    assert_eq!(
        rig.session.prompt("again", None).await,
        Ok(PromptAccepted::Started { turn: 2 })
    );
}

#[tokio::test(start_paused = true)]
async fn a_skipped_line_followed_by_the_result_ends_the_turn_as_usual() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.feed(ExternalAgentEvent::LineSkipped(SkippedLine {
        reason: SkippedLineReason::NotUtf8,
        bytes: 40,
    }))
    .await;
    let SessionStep::Folded(step) = rig.feed(completed("done")).await else {
        panic!("the result is folded")
    };
    assert!(step.turn_end.is_some());
    assert_eq!(rig.phase(), SessionPhase::Idle);
    // Idle, the stream may stay quiet for ever: no turn is lost.
    let quiet = tokio::time::timeout(GRACE * 3, rig.session.next_step()).await;
    assert!(quiet.is_err(), "an idle member waits: {quiet:?}");
}

#[tokio::test(start_paused = true)]
async fn silence_without_a_skipped_line_never_ends_a_turn() {
    // A long tool call is quiet: only a skipped line arms the grace.
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    let quiet = tokio::time::timeout(GRACE * 3, rig.session.next_step()).await;
    assert!(quiet.is_err(), "{quiet:?}");
    assert_eq!(rig.phase(), SessionPhase::Busy { turn: 1 });
}

#[tokio::test]
async fn the_agent_exiting_mid_turn_ends_the_member() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.session.follow_up("two").await.unwrap();
    rig.wire.end_output();
    assert_eq!(
        rig.step().await,
        Some(SessionStep::Ended {
            turn: Some(1),
            exit: ExternalAgentExit::Code(0),
        })
    );
    assert_eq!(rig.phase(), SessionPhase::Ended);
    assert_eq!(
        rig.wire.sent(),
        ["one"],
        "nothing is written to an exited agent"
    );
    assert_eq!(rig.step().await, None);
    assert_eq!(
        rig.session.prompt("three", None).await,
        Err(SessionRefusal::Ended)
    );
}

#[tokio::test]
async fn a_session_starts_its_agent_once_with_its_settings() {
    let rig = rig_with(false);
    assert_eq!(rig.phase(), SessionPhase::NotStarted);
    assert_eq!(
        rig.session.prompt("early", None).await,
        Err(SessionRefusal::NotStarted)
    );
    assert_eq!(rig.step().await, None);
    rig.session.start().await.unwrap();
    assert_eq!(
        rig.session.start().await,
        Err(SessionRefusal::AlreadyStarted)
    );
    assert_eq!(*rig.launcher.starts.lock().unwrap(), [settings().launch]);
    assert_eq!(rig.phase(), SessionPhase::Idle);

    let refused = rig_with(true);
    let error = refused.session.start().await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "claude not found on PATH (required for claude-code members)"
    );
    assert_eq!(refused.phase(), SessionPhase::Ended);
}

#[tokio::test]
async fn turn_ordinals_strictly_increase() {
    let rig = started().await;
    let mut last = 0;
    for round in 0..3 {
        let Ok(PromptAccepted::Started { turn }) = rig.session.prompt("go", None).await else {
            panic!("round {round} starts a turn")
        };
        assert!(turn > last, "{turn} after {last}");
        last = turn;
        rig.feed(completed("ok")).await;
    }
}

// Telemetry (#2284): every op records ids, kinds and sizes, and never the
// text it carried.
#[tokio::test]
async fn every_decision_and_effect_is_recorded_without_its_text() {
    let secret = "sk-ant-api03-PROMPTSECRETPROMPTSECRET";
    let rig = started().await;
    rig.session.prompt(secret, None).await.unwrap();
    rig.session.prompt(secret, None).await.unwrap_err();
    rig.session.follow_up(secret).await.unwrap();
    rig.feed(ExternalAgentEvent::AssistantBlock {
        message_id: "m1".into(),
        block: AssistantContent::ToolUse {
            id: "t1".into(),
            name: "Bash".into(),
            input: serde_json::json!({"command": secret}),
        },
    })
    .await;
    rig.feed(ExternalAgentEvent::LineSkipped(SkippedLine {
        reason: SkippedLineReason::NotUtf8,
        bytes: 9,
    }))
    .await;
    rig.feed(completed(secret)).await;
    rig.session.abort().await.unwrap();
    let bytes = secret.len();
    assert_eq!(
        rig.records.all(),
        [
            SessionRecord::Started,
            SessionRecord::PromptAccepted {
                accepted: PromptAccepted::Started { turn: 1 },
                bytes,
            },
            SessionRecord::PromptRefused {
                refusal: "busy",
                bytes,
            },
            SessionRecord::PromptAccepted {
                accepted: PromptAccepted::Queued { position: 1 },
                bytes,
            },
            SessionRecord::ToolCalled {
                turn: Some(1),
                tool: "Bash".into(),
            },
            SessionRecord::LineSkipped {
                turn: Some(1),
                bytes: 9,
            },
            SessionRecord::TurnEnded {
                turn: 1,
                outcome: "completed",
                duration_ms: Some(1200),
                cost_micro_usd: 0,
            },
            SessionRecord::FollowUpStarted { turn: 2, bytes },
            SessionRecord::Interrupted {
                turn: 2,
                cause: "abort",
            },
            SessionRecord::Aborted {
                turn: Some(2),
                dropped_follow_ups: 0,
            },
        ]
    );
    let logged = format!("{:?}", rig.records.all());
    assert!(!logged.contains("SECRET"), "{logged}");
}

#[tokio::test]
async fn a_refused_start_and_an_exit_are_recorded() {
    let refused = rig_with(true);
    refused.session.start().await.unwrap_err();
    assert_eq!(
        refused.records.all(),
        [SessionRecord::StartRefused { kind: "not_found" }]
    );

    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.wire.end_output();
    rig.step().await;
    assert_eq!(
        rig.records.kinds(),
        ["started", "prompt_accepted", "turn_ended", "ended"]
    );
    assert!(rig.records.all().contains(&SessionRecord::TurnEnded {
        turn: 1,
        outcome: "exited",
        duration_ms: None,
        cost_micro_usd: 0,
    }));
}

// Telemetry nit (#2287 review): a tool's name is recorded bounded, with
// only allowlisted characters, whatever the agent sent.
#[tokio::test]
async fn a_recorded_tool_name_is_bounded_and_allowlisted() {
    let rig = started().await;
    rig.session.prompt("one", None).await.unwrap();
    for name in [
        "mcp__board__board_inbox".to_string(),
        format!("Bash\u{1b}[31m {}", "x".repeat(200)),
        "é".repeat(40),
    ] {
        rig.feed(ExternalAgentEvent::AssistantBlock {
            message_id: "m1".into(),
            block: AssistantContent::ToolUse {
                id: "t1".into(),
                name,
                input: serde_json::json!({}),
            },
        })
        .await;
    }
    let tools: Vec<String> = rig
        .records
        .all()
        .into_iter()
        .filter_map(|record| match record {
            SessionRecord::ToolCalled { tool, .. } => Some(tool),
            _ => None,
        })
        .collect();
    assert_eq!(tools[0], "mcp__board__board_inbox");
    assert_eq!(tools[1], format!("Bash??31m?{}", "x".repeat(54)));
    assert_eq!(tools[2], "?".repeat(40), "one ? per character");
    for tool in &tools {
        assert!(tool.len() <= crate::application::external_agent::dto::TOOL_NAME_RECORD_BYTES);
    }
}

// A result names the user turns it consumed (claude's
// `user_message_uuids`): a steer folded mid-turn is answered by the turn's
// one result.
#[tokio::test]
async fn a_result_naming_the_steer_ends_the_steered_turn() {
    let rig = named_started().await;
    rig.session.prompt("one", None).await.unwrap();
    rig.session.steer("two").await.unwrap();
    let SessionStep::Folded(step) = rig.feed(answered(&["u2", "u3"], "completed", "done")).await
    else {
        panic!("the result is folded")
    };
    assert!(step.turn_end.is_some());
    assert_eq!(rig.phase(), SessionPhase::Idle);
}
