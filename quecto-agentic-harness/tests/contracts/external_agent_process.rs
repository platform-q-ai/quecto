//! Contract for [`ExternalAgentProcess`] (#2286), proven on the production
//! `ClaudeCodeProcess` against the mock `claude`: several turns over one
//! process, an API-error turn that leaves the process running, a clean
//! exit at input EOF, a bounded stderr, and no turn after the input is
//! closed. The events fold through the capability's `Projector`, which the
//! session tells of every spawn (`process_started`).
use quecto::application::external_agent::dto::{
    EXTERNAL_AGENT_STDERR_TAIL_BYTES, ExternalAgentExit, ExternalAgentInputError,
};
use quecto::application::external_agent::ports::ExternalAgentProcess;
use quecto::application::external_agent::projection::Projector;
use quecto::domain::external_agent::turn::TurnEnd;

use crate::mock_claude::{BOUND, MockClaudeRig, events_to_turn_end, fixture};

async fn exit_of(process: &dyn ExternalAgentProcess) -> ExternalAgentExit {
    process.close_input().await;
    tokio::time::timeout(BOUND, process.exited())
        .await
        .expect("exit is bounded")
}

/// Send `text`, fold the turn, and return how it ended.
async fn turn(
    projector: &mut Projector,
    process: &dyn ExternalAgentProcess,
    text: &str,
) -> TurnEnd {
    projector.record_user_turn(text);
    process
        .send_user_turn(text)
        .await
        .expect("the turn is written");
    let mut end = None;
    for event in events_to_turn_end(process).await {
        if let Some(outcome) = projector.apply(&event).turn_end {
            end = Some(outcome.end);
        }
    }
    end.expect("the result ends the turn")
}

#[tokio::test]
async fn two_turns_over_one_process_then_eof_exits_cleanly() {
    let rig = MockClaudeRig::replaying(&fixture("rt"));
    let process = rig.start("m1").await;
    let mut projector = Projector::new();
    projector.process_started();
    assert_eq!(
        turn(&mut projector, process.as_ref(), "first").await,
        TurnEnd::Completed
    );
    assert_eq!(
        turn(&mut projector, process.as_ref(), "second").await,
        TurnEnd::Completed
    );
    assert_eq!(exit_of(process.as_ref()).await, ExternalAgentExit::Code(0));
    let after = tokio::time::timeout(BOUND, process.next_event())
        .await
        .expect("the stream's end is bounded");
    assert_eq!(after, None, "no event follows the process's end");
}

/// A not-logged-in (API error) turn leaves the process taking turns. The
/// real-CLI behaviour this pins is spike #2264's surprise 7: `claude -p`
/// answers an authentication failure with an error `result` and keeps
/// reading its input. The fixture is synthesized from it: one process
/// keeps one `apiKeySource` (`none`) across both turns, as the real CLI
/// does; the second turn completing is the mock's replay, not a claim that
/// the CLI recovers without a credential.
#[tokio::test]
async fn an_api_error_turn_does_not_end_the_process() {
    let rig = MockClaudeRig::replaying(&fixture("not_logged_in"));
    let process = rig.start("m1").await;
    let mut projector = Projector::new();
    projector.process_started();
    let first = turn(&mut projector, process.as_ref(), "first").await;
    assert!(matches!(first, TurnEnd::Failed(_)), "{first:?}");
    assert_eq!(
        turn(&mut projector, process.as_ref(), "second").await,
        TurnEnd::Completed,
        "the process took a second turn after the API error"
    );
    assert_eq!(exit_of(process.as_ref()).await, ExternalAgentExit::Code(0));
}

#[tokio::test]
async fn stderr_is_bounded() {
    let root = tempfile::tempdir().unwrap();
    let scenario = root.path().join("noisy.jsonl");
    let fill = EXTERNAL_AGENT_STDERR_TAIL_BYTES * 2 + 17;
    std::fs::write(
        &scenario,
        format!(
            "@stderr first-words\n@stderr-fill {fill}\n@stderr last-words\n\
             {{\"type\": \"result\", \"subtype\": \"success\", \"is_error\": false}}\n"
        ),
    )
    .unwrap();
    let rig = MockClaudeRig::replaying(&scenario);
    let process = rig.start("m1").await;
    process.send_user_turn("go").await.unwrap();
    events_to_turn_end(process.as_ref()).await;
    assert!(exit_of(process.as_ref()).await.is_clean());
    // No polling: the exit is returned once stderr is read to its end.
    let tail = process.stderr_tail();
    assert!(
        tail.ends_with("last-words"),
        "the last words are in: {tail:?}"
    );
    assert!(
        tail.len() <= EXTERNAL_AGENT_STDERR_TAIL_BYTES,
        "{} bytes kept",
        tail.len()
    );
    assert!(
        !tail.contains("first-words"),
        "the oldest bytes are dropped"
    );
}

#[tokio::test]
async fn no_turn_is_written_after_the_input_is_closed() {
    let rig = MockClaudeRig::replaying(&fixture("rt"));
    let process = rig.start("m1").await;
    process.close_input().await;
    process.close_input().await;
    assert_eq!(
        process.send_user_turn("late").await,
        Err(ExternalAgentInputError::Closed)
    );
    let exit = tokio::time::timeout(BOUND, process.exited())
        .await
        .expect("exit is bounded");
    assert_eq!(exit, ExternalAgentExit::Code(0));
}

#[tokio::test]
async fn a_restarted_process_is_a_new_process() {
    let rig = MockClaudeRig::replaying(&fixture("rt"));
    let mut projector = Projector::new();
    for _ in 0..2 {
        let process = rig.start("m1").await;
        projector.process_started();
        assert_eq!(
            turn(&mut projector, process.as_ref(), "first").await,
            TurnEnd::Completed
        );
        assert_eq!(exit_of(process.as_ref()).await, ExternalAgentExit::Code(0));
    }
    // Each process's cumulative cost restarted from zero: the session's
    // total is both first turns', not a drop the ledger would ignore.
    // rt's first turn reports a cumulative $0.044864.
    let totals = projector.session_totals();
    assert_eq!(totals.cost_micro_usd, 2 * 44_864, "{totals:?}");
    assert!(
        projector
            .last_turn()
            .is_some_and(|turn| turn.warnings.is_empty()),
        "{:?}",
        projector.last_turn()
    );
}

/// `exited()` consumes no output: a session that reads events until the
/// process exits, then reads what is left, receives every event, the
/// turn's `result` included (#2286 review round 3).
#[tokio::test]
async fn waiting_for_the_exit_consumes_no_event() {
    let root = tempfile::tempdir().unwrap();
    let scenario = root.path().join("flood.jsonl");
    let thinking =
        "{\"type\": \"system\", \"subtype\": \"thinking_tokens\", \"estimated_tokens\": 5}\n";
    std::fs::write(
        &scenario,
        format!(
            "{}{{\"type\": \"result\", \"subtype\": \"success\", \"is_error\": false}}\n",
            thinking.repeat(200)
        ),
    )
    .unwrap();
    let rig = MockClaudeRig::replaying(&scenario);
    let process = rig.start("m1").await;
    process.send_user_turn("go").await.unwrap();
    process.close_input().await;
    let mut events = Vec::new();
    let exit = tokio::time::timeout(BOUND, async {
        let exit = loop {
            tokio::select! {
                event = process.next_event() => match event {
                    Some(event) => events.push(event),
                    None => break process.exited().await,
                },
                exit = process.exited() => break exit,
            }
        };
        while let Some(event) = process.next_event().await {
            events.push(event);
        }
        exit
    })
    .await
    .expect("the loop ends within the bound");
    assert_eq!(exit, ExternalAgentExit::Code(0));
    assert_eq!(events.len(), 201, "every event reached the session");
}
